# Shell helpers for ReaSock. Needs websocat and jq.
#
#   source examples/shell/reasock.sh
#   reasock tracks.list
#   reasock items.list '{"limit": 5}'
#   reasock action.run '{"command": 40044}'    # Transport: Play/stop
#   reasock_watch                              # follow events until Ctrl+C

# Port and token of the most recently started REAPER.
_reasock_instance() {
  local instance
  instance=$(ls -t "$HOME"/.reasock/instances/*.json 2>/dev/null | head -1)
  if [ -z "$instance" ]; then
    echo "no running REAPER found in ~/.reasock/instances" >&2
    return 1
  fi
  REASOCK_PORT=$(jq -r .port "$instance")
  REASOCK_TOKEN=$(jq -r .token "$instance")
}

_reasock_initialize() {
  jq -nc --arg token "$REASOCK_TOKEN" '{jsonrpc: "2.0", id: 0, method: "initialize",
    params: {protocolVersion: "0.3.0", client: {name: "shell"}, token: $token}}'
}

# reasock <method> [params-json]: calls one method and prints its result.
# initialize and the call travel as one JSON-RPC batch, so websocat can send
# one message, read one reply, and exit (-1); -n keeps the connection open
# after stdin ends, until that reply arrives.
reasock() {
  _reasock_instance || return 1
  local params=${2:-'{}'}
  jq -nc --argjson init "$(_reasock_initialize)" --arg method "$1" --argjson params "$params" \
    '[$init, {jsonrpc: "2.0", id: 1, method: $method, params: $params}]' \
    | websocat -n -1 "ws://127.0.0.1:$REASOCK_PORT" \
    | jq '.[] | select(.id == 1) | if .error then {error: .error} else .result end'
}

# reasock_watch [event...]: prints notifications until Ctrl+C. Default: all events.
reasock_watch() {
  _reasock_instance || return 1
  local events
  if [ $# -gt 0 ]; then events=$(printf '%s\n' "$@" | jq -R . | jq -sc .); else
    events='["transport.changed","project.changed","project.edited","items.selectionChanged","tracks.selectionChanged"]'
  fi
  {
    jq -nc --argjson init "$(_reasock_initialize)" --argjson events "$events" \
      '[$init, {jsonrpc: "2.0", id: 1, method: "events.subscribe", params: {events: $events}}]'
    cat  # keeps the connection open until Ctrl+C
  } | websocat "ws://127.0.0.1:$REASOCK_PORT" \
    | jq -c --unbuffered 'if type == "object" and .method then {event: .method, params} else empty end'
}
