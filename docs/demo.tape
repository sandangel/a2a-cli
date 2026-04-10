# agc — Agent CLI Demo
# Run: vhs docs/demo.tape
#
# Requires: vhs (https://github.com/charmbracelet/vhs)
#           agc binary on PATH (make install)
#           A registered demo agent (or mock below)

Output docs/demo.mp4
Output docs/demo.gif

Set Shell "bash"
Set FontSize 18
Set Width 900
Set Height 520
Set TypingSpeed 1ms
Set Padding 30
Set LineHeight 1.3

# ── Setup (hidden) ──
Hide
Type "export PATH=$PWD/bin:$PATH" Enter
Sleep 500ms

# Mock agent server for deterministic demo
Type "function _mock_agc_send() {" Enter
Type `  echo '{"agent":"demo","agent_url":"http://localhost:8080","type":"task_completed","text":"The capital of France is Paris.","task_id":"task-abc123","state":"TASK_STATE_COMPLETED"}'` Enter
Type "}" Enter
Type "alias agc-send-demo='_mock_agc_send'" Enter

# Pre-register a demo agent alias
Type "agc agent add demo http://localhost:8080 --description 'Demo agent'" Enter
Sleep 500ms
Type "agc agent use demo" Enter
Sleep 500ms
Type "clear" Enter
Sleep 1s
Show

# ╔══════════════════════════════════════╗
# ║         ASCII ART INTRO              ║
# ╚══════════════════════════════════════╝

Hide
Type "./scripts/show-art.sh art/intro.txt" Enter
Sleep 1s
Show
Sleep 3s

# ── Scene 1: What is AGC? ──
Hide
Type "./scripts/show-art.sh art/scene1.txt" Enter
Sleep 1s
Show
Sleep 4s

# ── Scene 2: Register agents ──
Hide
Type "./scripts/show-art.sh art/scene2.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc agent add prod  https://agent.example.com --description 'Production'" Enter
Sleep 1s
Type "agc agent add local http://localhost:8080     --description 'Local dev'" Enter
Sleep 1s
Type "agc agent list" Enter
Sleep 3s

# ── Scene 2b: Talk to an agent ──
Hide
Type "./scripts/show-art.sh art/scene2b.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc send 'What is the capital of France?'" Enter
Sleep 3s

# ── Scene 3: Inspect agent card ──
Hide
Type "./scripts/show-art.sh art/scene3.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc card" Enter
Sleep 3s

# ── Scene 3b: Normalized JSON ──
Hide
Type "./scripts/show-art.sh art/scene3b.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc --json send 'What is 2+2?'" Enter
Sleep 2s
Type "agc --json send 'Summarise' | jq -r .text" Enter
Sleep 2s

# ── Scene 4: Send a message ──
Hide
Type "./scripts/show-art.sh art/scene4.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc send 'Explain the A2A protocol in one sentence'" Enter
Sleep 3s

Type "agc send --context-id ctx-abc 'Follow up question'" Enter
Sleep 3s

# ── Scene 5: Authenticate ──
Hide
Type "./scripts/show-art.sh art/scene5.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc auth status" Enter
Sleep 2s

Type "# agc auth login  (auto-detects Device Code, PKCE, etc.)" Enter
Sleep 2s

# ── Scene 6: Manage tasks ──
Hide
Type "./scripts/show-art.sh art/scene6.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc task list" Enter
Sleep 2s
Type "agc task list --status TASK_STATE_WORKING" Enter
Sleep 2s
Type "agc task get --id task-abc123" Enter
Sleep 2s

# ── Scene 7: Parallel multi-agent ──
Hide
Type "./scripts/show-art.sh art/scene7.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc --agent prod --agent local send 'Health check?'" Enter
Sleep 3s
Type "agc --all send 'Status?' | jq -r '\"[\\(.agent)] \\(.text)\"'" Enter
Sleep 3s

# ── Scene 8: Generate skills ──
Hide
Type "./scripts/show-art.sh art/scene8.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc generate-skills --output-dir skills/" Enter
Sleep 3s
Type "ls skills/" Enter
Sleep 2s

# ── Scene 9: JSON for AI tools ──
Hide
Type "./scripts/show-art.sh art/scene9.txt" Enter
Sleep 1s
Show
Sleep 1s

Type "agc --json send 'Run analysis' | jq '{answer: .text, done: (.type == \"task_completed\")}'" Enter
Sleep 3s

# ── Features ──
Hide
Type "./scripts/show-art.sh art/features.txt" Enter
Sleep 1s
Show
Sleep 4s

# ╔══════════════════════════════════════╗
# ║         OUTRO                        ║
# ╚══════════════════════════════════════╝

Hide
Type "./scripts/show-art.sh art/outro.txt" Enter
Sleep 1s
Show
Sleep 3s
