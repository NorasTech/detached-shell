#!/bin/bash

echo "========================================"
echo "Testing PTY State Preservation Fix"
echo "========================================"
echo ""

# Clean up any existing sessions
echo "Step 1: Cleaning up existing sessions..."
./target/debug/nds list 2>/dev/null | grep -E '^[a-f0-9]{8}' | awk '{print $1}' | while read id; do
    ./target/debug/nds kill "$id" 2>/dev/null
done

echo "Step 2: Creating new detached session..."
SESSION_OUTPUT=$(./target/debug/nds new)
echo "$SESSION_OUTPUT"
SESSION_ID=$(echo "$SESSION_OUTPUT" | grep "Created session:" | awk '{print $3}')

if [ -z "$SESSION_ID" ]; then
    echo "ERROR: Failed to create session"
    exit 1
fi

echo ""
echo "Step 3: Session created with ID: $SESSION_ID"
echo ""

# Create expect script for automated testing
cat > /tmp/test_session.expect << EOF
#!/usr/bin/expect -f

set timeout 10
set session_id "$SESSION_ID"

# First attachment
spawn ./target/debug/nds attach \$session_id

# Wait for attachment message
expect {
    "Attached to session" { 
        send_user "\n>>> Successfully attached to session\n"
    }
    timeout {
        send_user "\n>>> ERROR: Failed to attach\n"
        exit 1
    }
}

# Set up test environment
send "cd /tmp\r"
expect "tmp"

send "export TEST_VAR='persistent_value_123'\r"
expect "$"

send "echo 'Current value: '\$TEST_VAR\r"
expect "persistent_value_123"

send "echo 'test content' > test_state_file.txt\r"
expect "$"

send "ls -la test_state_file.txt\r"
expect "test_state_file.txt"

# Start a background job
send "for i in {1..100}; do echo \"Background: \\\$i\"; sleep 1; done &\r"
expect "Background"

send "jobs\r"
expect "Running"

# Detach with Ctrl+\
send_user "\n>>> Detaching from session...\n"
send "\034"
expect "Detached from session"

send_user "\n>>> Successfully detached!\n"
send_user "\n>>> Waiting 3 seconds before reattaching...\n"
sleep 3

# Reattach to the same session
send_user "\n>>> Reattaching to session...\n"
spawn ./target/debug/nds attach \$session_id

expect {
    "Attached to session" {
        send_user "\n>>> Successfully reattached\n"
    }
    timeout {
        send_user "\n>>> ERROR: Failed to reattach\n"
        exit 1
    }
}

# Verify state is preserved
send_user "\n>>> Verifying state preservation...\n"

send "pwd\r"
expect {
    "/tmp" {
        send_user ">>> Working directory preserved: OK\n"
    }
    timeout {
        send_user ">>> Working directory NOT preserved: FAILED\n"
    }
}

send "echo 'Current value: '\$TEST_VAR\r"
expect {
    "persistent_value_123" {
        send_user ">>> Environment variable preserved: OK\n"
    }
    timeout {
        send_user ">>> Environment variable NOT preserved: FAILED\n"
    }
}

send "cat test_state_file.txt\r"
expect {
    "test content" {
        send_user ">>> File content preserved: OK\n"
    }
    timeout {
        send_user ">>> File content NOT preserved: FAILED\n"
    }
}

send "jobs\r"
expect {
    "Running" {
        send_user ">>> Background job preserved: OK\n"
    }
    "Done" {
        send_user ">>> Background job completed (expected): OK\n"
    }
    timeout {
        send_user ">>> Background job status unknown: WARNING\n"
    }
}

# Clean up
send "\034"
expect "Detached"

send_user "\n>>> Test completed!\n"
EOF

chmod +x /tmp/test_session.expect

# Check if expect is available
if ! command -v expect &> /dev/null; then
    echo "WARNING: 'expect' command not found. Please install expect to run automated tests."
    echo ""
    echo "Manual test instructions:"
    echo "========================"
    echo "1. Run: ./target/debug/nds attach $SESSION_ID"
    echo "2. Inside the session, run these commands:"
    echo "   cd /tmp"
    echo "   export TEST_VAR='persistent_value_123'"
    echo "   echo 'test content' > test_state_file.txt"
    echo "   for i in {1..100}; do echo \"Background: \$i\"; sleep 1; done &"
    echo "3. Detach with Ctrl+\\"
    echo "4. Wait a few seconds"
    echo "5. Reattach: ./target/debug/nds attach $SESSION_ID"
    echo "6. Verify state with:"
    echo "   pwd              # Should show /tmp"
    echo "   echo \$TEST_VAR  # Should show 'persistent_value_123'"
    echo "   cat test_state_file.txt  # Should show 'test content'"
    echo "   jobs             # Should show background job"
    echo ""
    echo "Session ID for manual testing: $SESSION_ID"
else
    echo "Running automated test with expect..."
    echo "======================================"
    /tmp/test_session.expect
    
    # Clean up
    echo ""
    echo "Cleaning up test session..."
    ./target/debug/nds kill "$SESSION_ID" 2>/dev/null
    rm -f /tmp/test_state_file.txt /tmp/test_session.expect
fi

echo ""
echo "========================================"
echo "Test Complete"
echo "========================================"