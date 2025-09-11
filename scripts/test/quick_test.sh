#!/bin/bash

echo "Quick Buffer Test"
echo "================="
echo ""

# Kill existing sessions
./target/debug/nds list 2>/dev/null | grep -E '^[a-f0-9]{8}' | awk '{print $1}' | while read id; do
    ./target/debug/nds kill "$id" 2>/dev/null
done

# Create new session
SESSION_ID=$(./target/debug/nds new | grep "Created session:" | awk '{print $3}')
echo "Created session: $SESSION_ID"

# Use a simple Python script to interact with the session
cat > /tmp/test_buffer.py << 'EOF'
#!/usr/bin/env python3
import subprocess
import time
import sys

session_id = sys.argv[1]

# First attachment - send some commands
print("First attachment - sending commands...")
proc1 = subprocess.Popen(
    ["./target/debug/nds", "attach", session_id],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True
)

# Send commands
proc1.stdin.write("echo 'MARKER_START'\n")
proc1.stdin.write("cd /tmp\n")
proc1.stdin.write("echo 'Changed to /tmp directory'\n")
proc1.stdin.write("export BUFFER_TEST='value_preserved'\n")
proc1.stdin.write("echo 'Set BUFFER_TEST variable'\n")
proc1.stdin.write("echo 'MARKER_END'\n")
proc1.stdin.flush()

# Wait a bit
time.sleep(1)

# Detach
proc1.stdin.write("\034")  # Ctrl+\
proc1.stdin.close()
proc1.wait()

print("Detached. Waiting 2 seconds...")
time.sleep(2)

# Second attachment - check if we can see the previous output
print("Second attachment - checking buffered output...")
proc2 = subprocess.Popen(
    ["./target/debug/nds", "attach", session_id],
    stdin=subprocess.PIPE,
    stdout=subprocess.PIPE,
    stderr=subprocess.PIPE,
    text=True
)

# Just check the output
time.sleep(1)
proc2.stdin.write("pwd\n")
proc2.stdin.write("echo $BUFFER_TEST\n")
proc2.stdin.write("\034")  # Ctrl+\
proc2.stdin.close()

# Get output
output = proc2.stdout.read()
errors = proc2.stderr.read()

print("\n=== OUTPUT ===")
# Check for our markers and values
if "MARKER_START" in output and "MARKER_END" in output:
    print("✓ Buffered output preserved - markers found!")
else:
    print("✗ Buffered output NOT preserved - markers missing")

if "/tmp" in output:
    print("✓ Working directory preserved")
else:
    print("✗ Working directory NOT preserved")

if "value_preserved" in output:
    print("✓ Environment variable preserved")
else:
    print("✗ Environment variable NOT preserved")

print("\nFull output (last 50 lines):")
print("-----------------------------")
lines = output.split('\n')
for line in lines[-50:]:
    if line.strip():
        print(line)
EOF

chmod +x /tmp/test_buffer.py

# Run the test
echo ""
echo "Running buffer test..."
echo "======================"
python3 /tmp/test_buffer.py "$SESSION_ID"

# Cleanup
echo ""
echo "Cleaning up..."
./target/debug/nds kill "$SESSION_ID" 2>/dev/null
rm -f /tmp/test_buffer.py

echo ""
echo "Test complete!"