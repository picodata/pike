#!/bin/sh
# Test-only wrapper around picodata. A vshard wakeup query sent to an admin
# socket whose path ends with $PIKE_TEST_FAIL_SOCKET is replaced with a Lua
# error. This is how a failed wakeup looks in the admin console: exit code 0
# and a "- error: ..." line. All other calls go to picodata from PATH.
if [ "$1" = "admin" ]; then
    query=$(cat)
    case "$2" in
        *"$PIKE_TEST_FAIL_SOCKET")
            case "$query" in
                *wakeup*) query=$(printf '\\lua\nerror("injected wakeup failure")') ;;
            esac ;;
    esac
    printf '%s' "$query" | picodata "$@"
    exit $?
fi
exec picodata "$@"
