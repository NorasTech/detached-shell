#!/bin/bash
set -e

echo "Testing state preservation in detached shell..."

# Kill any existing sessions
echo "1. Cleaning up existing sessions..."
./target/debug/nds list | grep -v "^ID" | grep -v "^---" | grep -v "^Total:" | awk '{print $1}' | xargs -I {} ./target/debug/nds kill {} 2>/dev/null || true

# Create new session
echo -e "\n2. Creating new session..."
SESSION_ID=$(./target/debug/nds new | grep "Created session:" | awk '{print $3}')
echo "Session ID: $SESSION_ID"

# Create a test script that will run inside the session
cat > /tmp/test_commands.sh << 'EOF'
cd /tmp
export TEST_VAR="persistent_value_123"
echo "Set TEST_VAR=$TEST_VAR"
echo "Current directory: $(pwd)"
echo "Creating test file..."
echo "test content" > test_file_$$
echo "File created: test_file_$$"
echo "Running background job..."
(while true; do echo "Background job running at $(date)"; sleep 5; done) > bg_output_$$.log 2>&1 &
echo "Background job PID: $!"
EOF

echo -e "\n3. Attaching to session and running commands..."
echo "Commands will execute:"
cat /tmp/test_commands.sh
echo -e "\n[Note: Use Ctrl+\\ to detach after commands execute]"

# Note to user: They need to manually attach and run the commands
echo -e "\nPlease run these commands manually:"
echo "1. ./target/debug/nds attach $SESSION_ID"
echo "2. Source the test script: source /tmp/test_commands.sh"
echo "3. Detach with Ctrl+\\"
echo "4. Re-attach with: ./target/debug/nds attach $SESSION_ID"
echo "5. Verify state with: echo \$TEST_VAR && pwd && ls test_file_* && jobs"