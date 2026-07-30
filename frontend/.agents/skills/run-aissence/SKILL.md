---
name: run-aissence
description: Start and manage the Aissence local development environment, including the SvelteKit app, local Supabase stack, Mailpit, and Studio. Use when the user asks to run, start, open, restart, or stop Aissence locally, or needs the local testing URLs.
---

# Run Aissence

Run commands from the Aissence repository root.

## Start the environment

1. Start `pnpm dev:local` in a retained PTY session. Use
   `pnpm dev:local:open` only when the user explicitly wants the browser opened.
2. The command starts the local Supabase stack, which includes Mailpit and
   Studio, derives the browser-safe Supabase configuration, and starts
   SvelteKit on port 5173.
3. Wait until Vite reports that the app is ready.
4. Report the Aissence, Mailpit, and Supabase Studio URLs shown by the command,
   then leave the PTY session running.

If access to the Docker or OrbStack socket is denied by the sandbox, rerun the
same command with narrowly scoped escalation. Never print, copy, or persist
secret values from `supabase status`.

If port 5173 is occupied by an earlier Aissence process, stop that retained
session and retry. Do not terminate an unrelated process without user approval.

## Stop the environment

- Send Ctrl-C to the retained PTY to stop only the SvelteKit app. Supabase and
  Mailpit intentionally remain available for fast restarts.
- Run `pnpm local:stop` only when the user asks to stop the complete local
  environment.
