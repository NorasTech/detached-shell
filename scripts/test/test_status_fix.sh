#!/bin/bash

echo "============================================"
echo "Testing Status File Fix"
echo "============================================"
echo ""

# Create a test session
echo "1. Creating test session..."
SESSION_ID=$(nds new --no-attach 2>&1 | grep "Created session:" | awk '{print $3}')
echo "   Created: $SESSION_ID"
sleep 1

echo ""
echo "2. Test Instructions:"
echo "   a) Open Terminal 1 and attach: nds attach $SESSION_ID"
echo "   b) Start a REPL (python3, node, etc) or vim"
echo "   c) From Terminal 2, run: nds list"
echo ""
echo "Expected behavior:"
echo "✓ nds list shows correct client count"
echo "✓ NO 'nds:count' text appears in Terminal 1"
echo "✓ REPL/vim session is NOT disrupted"
echo "✓ Screen does NOT redraw unnecessarily"
echo ""
echo "This fix uses a status file instead of socket queries,"
echo "preventing any interference with active sessions."
echo ""
echo "Session ID: $SESSION_ID"
echo ""
echo "To clean up: nds kill $SESSION_ID"