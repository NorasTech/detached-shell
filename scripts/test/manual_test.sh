#!/bin/bash

echo "Manual State Preservation Test"
echo "=============================="
echo ""

# Create a session
OUTPUT=$(./target/debug/nds new)
SESSION_ID=$(echo "$OUTPUT" | grep "Created session:" | awk '{print $3}')

echo "Created session: $SESSION_ID"
echo ""
echo "INSTRUCTIONS:"
echo "1. Open a new terminal window"
echo "2. Navigate to: $(pwd)"
echo "3. Attach to session: ./target/debug/nds attach $SESSION_ID"
echo "4. Run these commands in the attached session:"
echo "   cd /tmp"
echo "   export MY_TEST_VAR=hello_world"
echo "   echo 'File content' > testfile.txt"
echo "   echo 'You should see this after reattach'"
echo "5. Detach with Ctrl+\\"
echo "6. Wait 2 seconds"
echo "7. Reattach: ./target/debug/nds attach $SESSION_ID"
echo "8. Verify:"
echo "   - You should see the 'You should see this after reattach' message"
echo "   - Run: pwd (should show /tmp)"
echo "   - Run: echo \$MY_TEST_VAR (should show hello_world)"
echo "   - Run: cat testfile.txt (should show 'File content')"
echo ""
echo "Session ID: $SESSION_ID"
echo ""
echo "To kill session when done: ./target/debug/nds kill $SESSION_ID"