#shellcheck shell=bash
# End-to-end specs for the MCP stdio protocol.

Describe 'once mcp'
  BeforeEach 'setup_workspace'
  AfterEach 'cleanup_workspace'

  It 'advertises and calls the evidence query tool over stdio'
    once exec -e PATH=/usr/bin:/bin -- /bin/sh -c 'printf mcp-evidence' >/dev/null 2>&1
    cat > "$WORKSPACE/mcp.jsonl" <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"once_query_evidence","arguments":{}}}
EOF

    When call /bin/sh -c 'cat "$1" | "$2" -C "$3" mcp' sh "$WORKSPACE/mcp.jsonl" "$ONCE_BIN" "$WORKSPACE"
    The status should be success
    The stdout should include '"name":"once_query_evidence"'
    The stdout should include 'once.evidence.v1'
    The stdout should include 'action_result'
    The stdout should include 'passed'
  End

  It 'validates and executes an annotated script through the cache'
    cat > "$WORKSPACE/build.sh" <<'SH'
#!/bin/sh
# once input "input.txt"
# once output "output.txt"
cp input.txt output.txt
SH
    printf '%s\n' 'hello from an annotated script' > "$WORKSPACE/input.txt"
    cat > "$WORKSPACE/mcp.jsonl" <<'EOF'
{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"once_validate_script","arguments":{"path":"build.sh"}}}
{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"once_exec_script","arguments":{"path":"build.sh"}}}
{"jsonrpc":"2.0","id":3,"method":"tools/call","params":{"name":"once_exec_script","arguments":{"path":"build.sh"}}}
EOF

    When call /bin/sh -c 'cat "$1" | "$2" -C "$3" mcp --allow-run' sh "$WORKSPACE/mcp.jsonl" "$ONCE_BIN" "$WORKSPACE"
    The status should be success
    The stdout should include '"valid":true'
    The stdout should include '"cache":"hit"'
    The stdout should include '"evidence_subject"'
    The path "$WORKSPACE/output.txt" should be file
    The contents of file "$WORKSPACE/output.txt" should equal 'hello from an annotated script'
  End

  It 'verifies scripts and retrieves the same retained report without execution permission'
    cat > "$WORKSPACE/build.sh" <<'SH'
#!/bin/sh
# once output "out.txt"
printf stable > out.txt
SH
    verify_then_query() {
      printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/call","params":{"name":"once_exec_script","arguments":{"path":"build.sh","verify_reproducible":true}}}' |
        "$ONCE_BIN" -C "$WORKSPACE" mcp --allow-run > "$SPEC_ROOT/verified.json" || return
      id=$(jq -r '.result.structuredContent.result.id' "$SPEC_ROOT/verified.json")
      printf '{"jsonrpc":"2.0","id":2,"method":"tools/call","params":{"name":"once_query_verification","arguments":{"id":"%s"}}}\n' "$id" |
        "$ONCE_BIN" -C "$WORKSPACE" mcp > "$SPEC_ROOT/queried.json" || return
      jq -S '.result.structuredContent.result' "$SPEC_ROOT/verified.json" > "$SPEC_ROOT/first.json"
      jq -S '.result.structuredContent.result' "$SPEC_ROOT/queried.json" > "$SPEC_ROOT/second.json"
      cmp "$SPEC_ROOT/first.json" "$SPEC_ROOT/second.json" || return
      jq -c '[.schema, .success, .artifacts_match, .trials]' "$SPEC_ROOT/second.json"
    }
    When call verify_then_query
    The status should be success
    The stdout should equal '["once.verification.v1",true,true,2]'
    The path "$WORKSPACE/out.txt" should not be exist
  End

  It 'keeps editing and execution tools out of the default catalog'
    printf '%s\n' '{"jsonrpc":"2.0","id":1,"method":"tools/list","params":{}}' > "$WORKSPACE/mcp.jsonl"

    When call /bin/sh -c 'cat "$1" | "$2" -C "$3" mcp | jq -r ".result.tools[].name"' sh "$WORKSPACE/mcp.jsonl" "$ONCE_BIN" "$WORKSPACE"
    The status should be success
    The stdout should include 'once_validate_script'
    The stdout should include 'once_validate_workspace'
    The stdout should not include 'once_exec_script'
    The stdout should not include 'once_run_tests'
    The stdout should not include 'once_apply_edit'
  End
End
