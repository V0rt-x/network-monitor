# Thin wrapper so `just check` and `npm run check` are the same gate suite.

default: check

# Run every quality gate from CLAUDE.md's commit contract.
check:
    npm run check

# Run the app in development mode.
dev:
    npm run tauri dev
