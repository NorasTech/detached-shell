#!/bin/bash

echo "============================================"
echo "NDS Final Test - Multi-Client Support"
echo "============================================"
echo ""

# Create a test session
echo "1. Creating test session..."
SESSION_ID=$(nds new --no-attach 2>&1 | grep "Created session:" | awk '{print $3}')
echo "   Created: $SESSION_ID"
sleep 1

echo ""
echo "2. Listing sessions (should show detached):"
nds list | grep "$SESSION_ID"

echo ""
echo "============================================"
echo "TEST INSTRUCTIONS:"
echo ""
echo "Terminal 1:"
echo "  nds attach $SESSION_ID"
echo ""
echo "Terminal 2 (while Terminal 1 is attached):"
echo "  nds attach $SESSION_ID"
echo ""
echo "Expected behaviors:"
echo "✓ Both terminals connect successfully"
echo "✓ Terminal 1 sees: [Another client connected to this session (total: 2)]"
echo "✓ Both terminals share the same session view"
echo "✓ nds list shows 'attached(2)' status"
echo "✓ Either terminal can detach (~d) without affecting the other"
echo "✓ No 'Broken pipe' error when detaching"
echo "✓ Terminal properly restores after detach"
echo "✓ Disconnection notification shown to remaining clients"
echo ""
echo "To verify client count:"
echo "  nds list"
echo ""
echo "To clean up after testing:"
echo "  nds kill $SESSION_ID"
echo ""
echo "Session ID: $SESSION_ID"