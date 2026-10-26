#!/usr/bin/env python3
"""Build the bundled API reference from openrpc.json.

Writes a self-contained page (no external assets, so it works offline from
REAPER's Data folder) plus a copy of the document:

    target/docs/reasock-api.html
    target/docs/openrpc.json

ReaPack installs both to <resource>/Data/reasock/, where ReaSock_OpenDocs()
and the "ReaSock: Open API documentation" action find the page.
"""

from __future__ import annotations

import argparse
import base64
import html
import json
import re
import shutil
from pathlib import Path

ROOT = Path(__file__).resolve().parent.parent
ASSETS = ROOT / "docs" / "assets"


def data_url(name: str, mime: str) -> str:
    """Embeds an asset, so the page needs nothing beside it."""
    return f"data:{mime};base64," + base64.b64encode((ASSETS / name).read_bytes()).decode()


def esc(text: str) -> str:
    return html.escape(str(text), quote=True)


# Names that `code` spans in descriptions link to, filled in by build().
LINKS: dict[str, str] = {}


def code_span(match: re.Match) -> str:
    text = match.group(1)
    target = LINKS.get(html.unescape(text))
    return f'<a href="#{target}"><code>{text}</code></a>' if target else f"<code>{text}</code>"


def prose(text: str | None) -> str:
    """Escapes text and renders `code` spans, linking known names; blank lines
    separate paragraphs."""
    if not text:
        return ""
    paragraphs = re.split(r"\n\s*\n", text.strip())
    rendered = []
    for paragraph in paragraphs:
        body = esc(" ".join(line.strip() for line in paragraph.splitlines()))
        body = re.sub(r"`([^`]+)`", code_span, body)
        rendered.append(f"<p>{body}</p>")
    return "\n".join(rendered)


def anchor(kind: str, name: str) -> str:
    return f"{kind}-" + re.sub(r"[^A-Za-z0-9]+", "-", name).strip("-")


def type_of(schema: dict | bool | None) -> str:
    """A compact, linked rendering of a JSON Schema type."""
    if schema is None or schema is True or schema == {}:
        return "any"
    if not isinstance(schema, dict):
        return "never"
    if "$ref" in schema:
        name = schema["$ref"].rsplit("/", 1)[-1]
        return f'<a href="#{anchor("schema", name)}">{esc(name)}</a>'
    if "allOf" in schema and len(schema["allOf"]) == 1:
        return type_of(schema["allOf"][0])
    for key in ("anyOf", "oneOf"):
        if key in schema:
            return " | ".join(type_of(option) for option in schema[key])
    if "enum" in schema:
        return " | ".join(f"<code>{esc(json.dumps(value))}</code>" for value in schema["enum"])
    if "const" in schema:
        return f"<code>{esc(json.dumps(schema['const']))}</code>"
    kinds = schema.get("type", "any")
    kinds = kinds if isinstance(kinds, list) else [kinds]
    rendered = []
    for kind in kinds:
        if kind == "array":
            rendered.append(f"{type_of(schema.get('items'))}[]")
        elif kind == "object" and isinstance(schema.get("additionalProperties"), dict):
            rendered.append(f"map of {type_of(schema['additionalProperties'])}")
        else:
            rendered.append(esc(kind))
    return " | ".join(rendered)


def constraints(schema: dict) -> str:
    notes = []
    for key, label in (
        ("minimum", "min"),
        ("maximum", "max"),
        ("minLength", "min length"),
        ("maxLength", "max length"),
        ("minItems", "min items"),
        ("maxItems", "max items"),
        ("pattern", "pattern"),
        ("default", "default"),
    ):
        if key in schema:
            notes.append(f"{label} <code>{esc(json.dumps(schema[key]) if key != 'pattern' else schema[key])}</code>")
    return ", ".join(notes)


def fields_table(rows: list[tuple[str, dict, bool, str | None]]) -> str:
    if not rows:
        return "<p class=muted>None.</p>"
    body = []
    for name, schema, required, description in rows:
        notes = constraints(schema)
        detail = prose(description) + (f'<p class="muted">{notes}</p>' if notes else "")
        flag = "" if required else ' <span class="optional">optional</span>'
        kind = type_of(schema)
        if not required:
            # "optional" already says it can be left out.
            kind = kind.removesuffix(" | null")
        body.append(
            f"<tr><td><code>{esc(name)}</code>{flag}</td>"
            f"<td>{kind}</td><td>{detail}</td></tr>"
        )
    return (
        "<table><thead><tr><th>Name</th><th>Type</th><th>Description</th></tr></thead>"
        f"<tbody>{''.join(body)}</tbody></table>"
    )


def descriptor_rows(params: list[dict]) -> list[tuple[str, dict, bool, str | None]]:
    return [
        (param["name"], param.get("schema", {}), param.get("required", False), param.get("description"))
        for param in params
    ]


def schema_rows(schema: dict) -> list[tuple[str, dict, bool, str | None]]:
    required = set(schema.get("required", []))
    rows = []
    for name, field in schema.get("properties", {}).items():
        field = dict(field)
        description = field.pop("description", None)
        rows.append((name, field, name in required, description))
    return rows


def render_method(method: dict, errors: dict) -> str:
    parts = [f'<section id="{anchor("method", method["name"])}" class="entry">']
    parts.append(f"<h3><code>{esc(method['name'])}</code></h3>")
    parts.append(f"<p class=summary>{esc(method.get('summary', ''))}</p>")
    parts.append(prose(method.get("description")))
    parts.append("<h4>Params</h4>" + fields_table(descriptor_rows(method.get("params", []))))
    if "result" in method:
        parts.append(f"<h4>Result</h4><p>{type_of(method['result'].get('schema'))}</p>")
    refs = [error["$ref"].rsplit("/", 1)[-1] for error in method.get("errors", [])]
    if refs:
        items = "".join(
            f'<li><a href="#{anchor("error", name)}">{esc(name)}</a> '
            f"<code>{errors.get(name, {}).get('code', '')}</code></li>"
            for name in refs
        )
        parts.append(f"<h4>Errors</h4><ul>{items}</ul>")
    for example in method.get("examples", []):
        params = {param["name"]: param["value"] for param in example.get("params", [])}
        result = example.get("result", {}).get("value")
        parts.append(
            f"<h4>Example: {esc(example.get('name', ''))}</h4>"
            f"<pre>{esc(json.dumps({'method': method['name'], 'params': params}, indent=2))}"
            f"\n→ {esc(json.dumps(result, indent=2))}</pre>"
        )
    parts.append("</section>")
    return "\n".join(parts)


def render_notification(notification: dict) -> str:
    return "\n".join([
        f'<section id="{anchor("notification", notification["name"])}" class="entry">',
        f"<h3><code>{esc(notification['name'])}</code></h3>",
        f"<p class=summary>{esc(notification.get('summary', ''))}</p>",
        prose(notification.get("description")),
        "<h4>Params</h4>" + fields_table(descriptor_rows(notification.get("params", []))),
        "</section>",
    ])


def render_function(function: dict) -> str:
    params = ", ".join(f"{esc(param['type'])} {esc(param['name'])}" for param in function["params"])
    return "\n".join([
        f'<section id="{anchor("lua", function["name"])}" class="entry">',
        f"<h3><code>{esc(function['name'])}</code></h3>",
        f"<pre>{esc(function['returns'])} reaper.{esc(function['name'])}({params})</pre>",
        prose(function.get("description")),
        "</section>",
    ])


def render_schema(name: str, schema: dict) -> str:
    parts = [f'<section id="{anchor("schema", name)}" class="entry">', f"<h3><code>{esc(name)}</code></h3>"]
    parts.append(prose(schema.get("description")))
    if "properties" in schema:
        parts.append(fields_table(schema_rows(schema)))
    else:
        parts.append(f"<p>{type_of({k: v for k, v in schema.items() if k != 'description'})}</p>")
    parts.append("</section>")
    return "\n".join(parts)


# Tech Audio styling, after techaud.io and Dingus's build index: gray-900
# base, fuchsia accent, Karla for text and Roboto Slab for headings.
STYLE = """
@font-face { font-family: "Karla"; font-weight: 200 800; font-display: swap;
  src: url(KARLA) format("woff2"); }
@font-face { font-family: "Roboto Slab"; font-weight: 500; font-display: swap;
  src: url(ROBOTO_SLAB) format("woff2"); }
:root {
  color-scheme: dark;
  --ta-bg: #111827; --ta-surface: #1f2937; --ta-surface-raised: #374151; --ta-border: #4b5563;
  --ta-text: #d1d5db; --ta-text-muted: #9ca3af; --ta-heading: #f9fafb;
  --ta-accent: #d946ef; --ta-accent-hover: #e879f9; --ta-link: #93c5fd;
  --font-body: "Karla", ui-sans-serif, system-ui, sans-serif;
  --font-heading: "Roboto Slab", ui-serif, Georgia, serif;
  --font-mono: ui-monospace, SFMono-Regular, Menlo, Consolas, monospace;
}
* { box-sizing: border-box; }
body { margin: 0; font: 16px/1.6 var(--font-body); color: var(--ta-text); background: var(--ta-bg); }
.page { max-width: 1240px; margin: 0 auto; padding: 1.5rem 1.5rem 3rem; }
header.page-header { display: flex; align-items: center; gap: 1.25rem; padding-bottom: 1rem;
  border-bottom: 1px solid var(--ta-accent); margin-bottom: 1.5rem; }
header.page-header img { width: 84px; height: auto; }
.eyebrow { font-size: .875rem; margin: 0 0 .25rem; color: var(--ta-text-muted); }
.eyebrow a { color: var(--ta-accent); font-weight: 600; }
.eyebrow a:hover { color: var(--ta-accent-hover); }
h1, h2, h3 { font-family: var(--font-heading); font-weight: 500; color: var(--ta-heading); }
h1 { font-size: 2rem; margin: 0; line-height: 1.2; }
h2 { font-size: 1.5rem; margin: 3rem 0 1rem; padding-bottom: .35rem; border-bottom: 1px solid var(--ta-accent); }
h3 { font-size: 1.15rem; margin: 0 0 .25rem; }
h4 { margin: 1.1rem 0 .4rem; font-size: .75rem; text-transform: uppercase; letter-spacing: .08em;
  color: var(--ta-text-muted); font-weight: 600; }
a { color: var(--ta-link); text-decoration: none; }
a:hover { text-decoration: underline; }
.layout { display: grid; grid-template-columns: 15rem 1fr; gap: 2rem; align-items: start; }
@media (max-width: 860px) { .layout { grid-template-columns: 1fr; } nav.toc { position: static; max-height: none; } }
nav.toc { position: sticky; top: 1rem; max-height: calc(100vh - 2rem); overflow: auto;
  background: var(--ta-surface); border: 1px solid var(--ta-border); border-radius: .5rem;
  padding: .75rem 1rem; font-size: .85rem; box-shadow: 0 4px 6px rgb(0 0 0 / .15); }
nav.toc strong { display: block; margin: .9rem 0 .3rem; font-family: var(--font-heading);
  font-weight: 500; color: var(--ta-heading); }
nav.toc strong:first-child { margin-top: 0; }
nav.toc a { display: block; padding: .1rem 0; color: var(--ta-text-muted); }
nav.toc a:hover { color: var(--ta-link); text-decoration: none; }
.entry { background: var(--ta-surface); border: 1px solid var(--ta-border); border-radius: .5rem;
  padding: 1rem 1.25rem; margin-bottom: 1rem; box-shadow: 0 4px 6px rgb(0 0 0 / .2); }
code, pre { font-family: var(--font-mono); font-size: .85em; }
code { background: var(--ta-surface-raised); color: var(--ta-heading); padding: .1rem .35rem; border-radius: .25rem; }
pre { background: var(--ta-bg); border: 1px solid var(--ta-border); border-radius: .375rem;
  padding: .75rem 1rem; overflow: auto; color: var(--ta-heading); }
pre code { background: none; padding: 0; }
h3 code { font-size: 1em; background: none; padding: 0; }
a code { color: var(--ta-link); }
table { width: 100%; border-collapse: collapse; font-size: .9rem; }
th, td { text-align: left; vertical-align: top; padding: .45rem .6rem; border-bottom: 1px solid var(--ta-border); }
th { font-weight: 600; color: var(--ta-heading); }
td p { margin: 0 0 .25rem; }
.summary { font-size: 1.05rem; color: var(--ta-heading); margin: 0 0 .5rem; }
.muted { color: var(--ta-text-muted); }
.optional { font-size: .72rem; color: var(--ta-text-muted); background: var(--ta-surface-raised);
  border-radius: 999px; padding: 0 .45rem; margin-left: .25rem; }
footer { margin-top: 3rem; padding-top: 1rem; border-top: 1px solid var(--ta-border);
  font-size: .8rem; color: var(--ta-text-muted); }
footer a { color: var(--ta-accent); }
"""

FONT_NOTICE = """<!--
Karla (c) 2019 The Karla Project Authors, SIL Open Font License 1.1.
Roboto Slab (c) Google, Apache License 2.0.
Both are embedded below; their licenses are in licenses/ next to this page.
-->"""


def build(document: dict) -> str:
    info = document["info"]
    methods = document.get("methods", [])
    notifications = document.get("x-notifications", [])
    functions = document.get("x-reascript", [])
    schemas = document.get("components", {}).get("schemas", {})
    errors = document.get("components", {}).get("errors", {})
    LINKS.clear()
    LINKS.update({name: anchor("schema", name) for name in schemas})
    LINKS.update({m["name"]: anchor("method", m["name"]) for m in methods})
    LINKS.update({n["name"]: anchor("notification", n["name"]) for n in notifications})
    LINKS.update({f["name"]: anchor("lua", f["name"]) for f in functions})

    def nav_section(title: str, kind: str, names: list[str]) -> str:
        links = "".join(f'<a href="#{anchor(kind, name)}">{esc(name)}</a>' for name in names)
        return f"<strong>{esc(title)}</strong>{links}"

    nav = "".join([
        '<strong>Guide</strong><a href="#connecting">Connecting</a><a href="#reascript">ReaScript</a>',
        nav_section("Methods", "method", [m["name"] for m in methods]),
        nav_section("Notifications", "notification", [n["name"] for n in notifications]),
        nav_section("ReaScript functions", "lua", [f["name"] for f in functions]),
        nav_section("Types", "schema", list(schemas)),
        '<strong>Reference</strong><a href="#errors">Errors</a>',
    ])

    error_rows = "".join(
        f'<tr id="{anchor("error", name)}"><td><code>{esc(name)}</code></td>'
        f"<td><code>{error['code']}</code></td><td>{esc(error['message'])}</td></tr>"
        for name, error in errors.items()
    )
    standard = [
        (-32700, "Parse error: the message isn't JSON"),
        (-32600, "Invalid request: not a JSON-RPC 2.0 message"),
        (-32601, "Method not found"),
        (-32602, "Invalid params: wrong shape, or outside a documented constraint"),
        (-32603, "Internal error"),
    ]
    standard_rows = "".join(
        f"<tr><td>standard</td><td><code>{code}</code></td><td>{esc(text)}</td></tr>" for code, text in standard
    )
    server = (document.get("servers") or [{}])[0]
    style = STYLE.replace("KARLA", data_url("karla-latin-wght-normal.woff2", "font/woff2")).replace(
        "ROBOTO_SLAB", data_url("roboto-slab-latin-500-normal.woff2", "font/woff2")
    )
    logo = data_url("techaudio-logo.png", "image/png")

    return f"""<!doctype html>
<html lang="en"><head><meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<title>ReaSock {esc(info['version'])} API reference · Tech Audio</title>
{FONT_NOTICE}
<style>{style}</style></head>
<body><div class="page">
<header class="page-header">
<a href="https://techaud.io/"><img src="{logo}" alt="Tech Audio"></a>
<div><p class="eyebrow"><a href="https://techaud.io/">Tech Audio</a> · ReaSock</p>
<h1>API reference</h1>
<p class="muted" style="margin:.25rem 0 0">Protocol {esc(info['version'])} · OpenRPC {esc(document['openrpc'])}</p></div>
</header>
<div class="layout"><nav class="toc">{nav}</nav><main>
{prose(info.get('description'))}

<h2 id="connecting">Connecting</h2>
<p>Each running REAPER writes <code>~/.reasock/instances/&lt;pid&gt;.json</code> with the
<code>port</code> it listens on. Connect a WebSocket to <code>{esc(server.get('url', 'ws://127.0.0.1:{port}'))}</code>
and send JSON-RPC 2.0 text frames, starting with <code>initialize</code>:</p>
<pre>{esc(json.dumps({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": info['version'], "client": {"name": "myapp"}}}, indent=2))}</pre>
<p>Then call methods, and <code>events.subscribe</code> to receive notifications. The machine-readable
version of this reference is <code>openrpc.json</code>, next to this page, and <code>rpc.discover</code>
returns it from a running REAPER.</p>

<h2 id="reascript">ReaScript</h2>
<p>Scripts inside REAPER speak the same protocol through a local connection:</p>
<pre>local conn = reaper.ReaSock_Open("myscript")
local reply = reaper.ReaSock_Call(conn, "tracks.list", '{{"limit": 10}}')
-- poll from a reaper.defer loop for replies, events, and action.triggered:
local message = reaper.ReaSock_Poll(conn)   -- "" when nothing is waiting
reaper.ReaSock_Close(conn)</pre>

<h2>Methods</h2>
{''.join(render_method(method, errors) for method in methods)}

<h2>Notifications</h2>
<p>Sent by ReaSock without a request: <code>action.triggered</code> for actions a connection
registered, and the events it subscribed to.</p>
{''.join(render_notification(notification) for notification in notifications)}

<h2>ReaScript functions</h2>
{''.join(render_function(function) for function in functions)}

<h2>Types</h2>
{''.join(render_schema(name, schema) for name, schema in schemas.items())}

<h2 id="errors">Errors</h2>
<table><thead><tr><th>Name</th><th>Code</th><th>Meaning</th></tr></thead>
<tbody>{standard_rows}{error_rows}</tbody></table>
</main></div>
<footer>ReaSock is free software from <a href="https://techaud.io/">Tech Audio</a>, licensed under
the GNU LGPL 3.0 or later. Set in Karla (SIL OFL 1.1) and Roboto Slab (Apache 2.0).</footer>
</div></body></html>
"""


def main() -> None:
    parser = argparse.ArgumentParser(description=__doc__)
    parser.add_argument("--openrpc", type=Path, default=ROOT / "openrpc.json")
    parser.add_argument("--out", type=Path, default=ROOT / "target" / "docs")
    args = parser.parse_args()

    document = json.loads(args.openrpc.read_text(encoding="utf-8"))
    args.out.mkdir(parents=True, exist_ok=True)
    (args.out / "reasock-api.html").write_text(build(document), encoding="utf-8")
    shutil.copyfile(args.openrpc, args.out / "openrpc.json")
    licenses = args.out / "licenses"
    licenses.mkdir(exist_ok=True)
    for name in ("karla-LICENSE.txt", "roboto-slab-LICENSE.txt"):
        shutil.copyfile(ASSETS / name, licenses / name)
    print(f"wrote {args.out / 'reasock-api.html'}")


if __name__ == "__main__":
    main()
