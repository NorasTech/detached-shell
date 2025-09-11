#!/bin/bash

# Test script for concurrent attachments

echo "Testing NDS concurrent attachment support..."
echo "=========================================="

# Create a new session
echo "1. Creating a new test session..."
SESSION_ID=$(nds new --no-attach 2>&1 | grep "Created session:" | awk '{print $3}')
echo "   Created session: $SESSION_ID"

# Wait for session to initialize
sleep 1

# Test 1: Attach from first terminal
echo ""
echo "2. Testing first attachment..."
echo "   Run this in Terminal 1:"
echo "   nds attach $SESSION_ID"
echo ""
echo "3. While attached in Terminal 1, run this in Terminal 2:"
echo "   nds attach $SESSION_ID"
echo ""
echo "4. You should see:"
echo "   - Terminal 1 gets a notification about Terminal 2 connecting"
echo "   - Both terminals can see the same session output"
echo "   - Either terminal can detach without affecting the other"
echo ""
echo "5. Test detaching:"
echo "   - In Terminal 1: Press Enter then type ~d to detach"
echo "   - Terminal 2 should remain connected"
echo "   - You should not see 'Broken pipe' error"
echo ""
echo "Session ID for testing: $SESSION_ID"
echo ""
echo "To clean up after testing:"
echo "nds kill $SESSION_ID"