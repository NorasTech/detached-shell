#!/bin/bash

# Test script for NDS (Noras Detached Shell)

echo "=== NDS Test Script ==="
echo

# Build the project
echo "Building NDS..."
cargo build --release 2>/dev/null
if [ $? -ne 0 ]; then
    echo "Build failed!"
    exit 1
fi

NDS="./target/release/nds"

# Clean up any existing sessions
echo "Cleaning up existing sessions..."
rm -rf ~/.nds/*

# Test 1: Create a new session
echo
echo "Test 1: Creating a new detached session..."
SESSION_OUTPUT=$($NDS new)
SESSION_ID=$(echo "$SESSION_OUTPUT" | grep "Created session:" | cut -d' ' -f3)
echo "Created session: $SESSION_ID"

# Test 2: List sessions
echo
echo "Test 2: Listing sessions..."
$NDS list

# Test 3: Get session info
echo
echo "Test 3: Getting session info..."
$NDS info $SESSION_ID

# Test 4: Create multiple sessions
echo
echo "Test 4: Creating two more sessions..."
$NDS new > /dev/null
$NDS new > /dev/null
echo "Total sessions:"
$NDS list | tail -1

# Test 5: Kill a session
echo
echo "Test 5: Killing the first session..."
$NDS kill $SESSION_ID
echo "Remaining sessions:"
$NDS list | tail -1

# Test 6: Clean up
echo
echo "Test 6: Cleaning all sessions..."
for id in $($NDS list | grep -E '^[a-f0-9]{8}' | awk '{print $1}'); do
    $NDS kill $id 2>/dev/null
done
echo "Final session count:"
$NDS list | tail -1

echo
echo "=== All tests completed successfully ==="