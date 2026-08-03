# aissence

A SvelteKit and Supabase application for publishing the prompt history behind an
AI-assisted software project. Creators upload a `.aissence` file, add project
context and screenshots, preview the result privately, and publish a public
ledger.

Public readers see one privacy-safe story and up to three representative beats
per commit; the public query never selects message rows. Active creator
subscribers additionally see every prompt summary and retained
`UserWrote.content`; assistant source text and edit diffs remain creator-only.

## Local development

Requirements:

- Node.js 22+
- pnpm 10+
- Docker or OrbStack for the local Supabase stack
- Supabase CLI

Install dependencies and start Supabase:

```sh
pnpm install
pnpm supabase:start
```

Copy `.env.example` to `.env` and fill it with the URL and publishable key shown
by `supabase status`. Then run:

```sh
pnpm dev
```

For the normal local workflow, one command starts Supabase, Mailpit, and
Aissence's frontend and Rust API, and derives the local URLs, database URL,
JWT secret, and publishable key automatically. Configure `OPENROUTER_API_KEY`
in the repository root `.env` (or export it) so the hosted summary worker can
start:

```sh
pnpm dev:local
```

The equivalent repository-root entry point is `scripts/dev.sh`.

Use `pnpm dev:local:open` to also open Aissence and Mailpit in the default
browser. Press Ctrl-C to stop the Aissence server; Supabase remains available
for the next run. Stop its containers explicitly with `pnpm local:stop`.

The local migrations create the database schema, audience-specific RLS
policies, private Storage buckets, durable summary jobs, and the manual
subscription helper. Browser uploads are validated and ingested by the Rust
server configured through `AISSENCE_API_URL`; the frontend does not write JSON
snapshots into Postgres itself.

## Supabase configuration

For a hosted project:

1. Link the project with the Supabase CLI.
2. Apply every migration in `supabase/migrations` in timestamp order.
3. Add the production URL and Vercel preview URLs to Auth redirect URLs.
4. Configure the email provider and magic-link template.
5. Set the environment variables from `.env.example` in Vercel. Never expose
   the service-role key through a `PUBLIC_` variable.

The application creates signed upload URLs for the private `aissence-staging`
and `project-media` buckets. Public project pages receive short-lived signed
screenshot URLs after their project visibility has been checked.

## Manual subscription grants

The payment/KYC provider is intentionally not connected in v1. Grant test access from a trusted
SQL/service-role session:

```sql
select public.grant_manual_subscription(
  '<subscriber auth.users id>'::uuid,
  '<creator profiles id>'::uuid,
  now() + interval '30 days'
);
```

Subscriptions apply to every current and future published project owned by that
creator. The same table has nullable external billing identifiers for the future
provider webhook.

## Validation

```sh
pnpm check
pnpm test:unit
pnpm build
pnpm test:e2e
supabase test db
```

The SQL tests require the local Supabase containers. They verify anonymous,
unrelated-reader, subscriber, and creator access to summaries and retained
source content.

## Deployment

The project uses `@sveltejs/adapter-vercel` and the Node.js runtime. Import and
ledger routes are server-rendered. Ledger responses use `private, no-store` and
`Vary: Cookie` because entitlement changes the response for the same URL.
