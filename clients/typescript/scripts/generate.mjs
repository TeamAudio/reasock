#!/usr/bin/env node
// Generates src/generated.ts from the repo's openrpc.json: an interface or
// type for every schema, and maps from method and notification names to
// their params and results. `--check` fails if the file is out of date.

import { readFileSync, writeFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const root = new URL("../", import.meta.url);
const openrpcPath = fileURLToPath(new URL("../../openrpc.json", root));
const outPath = fileURLToPath(new URL("src/generated.ts", root));
const doc = JSON.parse(readFileSync(openrpcPath, "utf8"));
const schemas = doc.components?.schemas ?? {};

const IDENTIFIER = /^[A-Za-z_$][A-Za-z0-9_$]*$/;
const key = (name) => (IDENTIFIER.test(name) ? name : JSON.stringify(name));
const refName = (ref) => ref.split("/").pop();

function comment(text, indent) {
  if (!text) return "";
  const lines = text.replace(/\*\//g, "* /").split("\n").map((line) => line.trim());
  return lines.length === 1
    ? `${indent}/** ${lines[0]} */\n`
    : `${indent}/**\n${lines.map((line) => `${indent} * ${line}`).join("\n")}\n${indent} */\n`;
}

function union(types) {
  const unique = [...new Set(types)];
  return unique.length === 1 ? unique[0] : unique.join(" | ");
}

/** A TypeScript type for a JSON Schema (the draft-07 subset schemars emits). */
function type(schema, indent = "") {
  if (schema === true || schema === undefined || schema === null) return "unknown";
  if (schema === false) return "never";
  if (schema.$ref) return refName(schema.$ref);
  if (schema.allOf?.length === 1) return type(schema.allOf[0], indent);
  const options = schema.anyOf ?? schema.oneOf;
  if (options) return union(options.map((option) => type(option, indent)));
  if (schema.enum) return union(schema.enum.map((value) => JSON.stringify(value)));
  if ("const" in schema) return JSON.stringify(schema.const);
  const kinds = Array.isArray(schema.type) ? schema.type : schema.type ? [schema.type] : [];
  if (kinds.length === 0) return schema.properties ? object(schema, indent) : "unknown";
  return union(
    kinds.map((kind) => {
      switch (kind) {
        case "integer":
        case "number":
          return "number";
        case "string":
        case "boolean":
        case "null":
          return kind;
        case "array": {
          const item = type(schema.items, indent);
          return /[|&]/.test(item) ? `(${item})[]` : `${item}[]`;
        }
        case "object":
          return object(schema, indent);
        default:
          return "unknown";
      }
    }),
  );
}

function object(schema, indent) {
  const properties = schema.properties ?? {};
  const names = Object.keys(properties);
  if (names.length === 0) {
    // Keys limited by a pattern: TypeScript can't check the pattern, so any string key.
    const patterns = Object.values(schema.patternProperties ?? {});
    if (patterns.length > 0) {
      const values = [...new Set(patterns.map((value) => type(value, indent)))].join(" | ");
      return `{ [key: string]: ${values} }`;
    }
    const values = schema.additionalProperties;
    if (values === false) return "Record<string, never>";
    return `{ [key: string]: ${type(values === undefined ? true : values, indent)} }`;
  }
  const required = new Set(schema.required ?? []);
  const inner = `${indent}  `;
  const fields = names.map((name) => {
    const property = properties[name];
    const optional = required.has(name) ? "" : "?";
    return `${comment(property.description, inner)}${inner}${key(name)}${optional}: ${type(property, inner)};`;
  });
  return `{\n${fields.join("\n")}\n${indent}}`;
}

/** An object type from OpenRPC content descriptors (method params). */
function descriptors(params, indent) {
  if (!params?.length) return "Record<string, never>";
  const inner = `${indent}  `;
  const fields = params.map((param) => {
    const optional = param.required ? "" : "?";
    return `${comment(param.description, inner)}${inner}${key(param.name)}${optional}: ${type(param.schema, inner)};`;
  });
  return `{\n${fields.join("\n")}\n${indent}}`;
}

const out = [
  "// Generated from openrpc.json by scripts/generate.mjs. Don't edit by hand:",
  "// run `npm run generate` after the protocol changes.",
  "",
  `/** The protocol version these types describe. */`,
  `export const PROTOCOL_VERSION = ${JSON.stringify(doc.info.version)};`,
  "",
];

for (const [name, schema] of Object.entries(schemas)) {
  out.push(comment(schema.description, "").trimEnd());
  const isInterface = schema.type === "object" && schema.properties && !schema.anyOf && !schema.oneOf;
  out.push(isInterface ? `export interface ${name} ${object(schema, "")}` : `export type ${name} = ${type(schema, "")};`);
  out.push("");
}

out.push("/** Every method, with its params and result. */", "export interface Methods {");
for (const method of doc.methods) {
  out.push(comment([method.summary, method.description].filter(Boolean).join("\n\n"), "  ").trimEnd());
  out.push(`  ${JSON.stringify(method.name)}: {`);
  out.push(`    params: ${descriptors(method.params, "    ")};`);
  out.push(`    result: ${type(method.result?.schema, "    ")};`);
  out.push("  };");
}
out.push("}", "");

out.push("/** Every notification the server sends, with its params. */", "export interface Notifications {");
for (const notification of doc["x-notifications"] ?? []) {
  out.push(comment(notification.summary, "  ").trimEnd());
  const params = notification["x-params"] ? type(notification["x-params"], "  ") : descriptors(notification.params, "  ");
  out.push(`  ${JSON.stringify(notification.name)}: ${params};`);
}
out.push("}", "");

out.push(
  "export type MethodName = keyof Methods;",
  "export type NotificationName = keyof Notifications;",
  "",
  "/** Application error codes, beyond JSON-RPC's standard ones. */",
  "export const ErrorCode = {",
  ...Object.entries(doc.components?.errors ?? {}).map(([name, error]) => `  ${name}: ${error.code},`),
  "} as const;",
  "",
);

const text = out.filter((line, index, all) => !(line === "" && all[index - 1] === "")).join("\n");

if (process.argv.includes("--check")) {
  const current = readFileSync(outPath, "utf8");
  if (current !== text) {
    console.error("src/generated.ts is out of date; run `npm run generate`.");
    process.exit(1);
  }
  console.log("src/generated.ts is up to date.");
} else {
  writeFileSync(outPath, text);
  console.log(`wrote ${outPath}`);
}
