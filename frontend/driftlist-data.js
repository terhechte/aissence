window.DRIFTLIST_DATA = {
  product: {
    name: "Driftlist",
    tagline: "Turn a pile of trip links into a plan you'll actually follow.",
    description: "Driftlist is where a trip starts as chaos — a dozen tabs, screenshots, and \"we should go here\" texts — and ends as a shared day-by-day itinerary. Drop in a link or photo, Driftlist figures out what it is and where it might fit, and your group drags things into place together.",
    url: "https://driftlist.app",
    repo: "driftlist/app",
    screenshots: [
      { label: "the pile — unsorted ideas", ratio: "4/3" },
      { label: "day-by-day board", ratio: "4/3" },
      { label: "shared map view", ratio: "4/3" },
      { label: "a card, opened", ratio: "4/3" },
      { label: "the public plan", ratio: "4/3" }
    ]
  },
  commits: [
    {
      hash: "3f9a0c1e2b4d6f8091a2b3c4d5e6f708192a3b4c",
      date: "2026-06-18T09:12:00Z",
      title: "Scaffold the trip board",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 612, linesDeleted: 4, filesChanged: 14, minutesSincePrompt: 6, tokensIn: 95500, tokensOut: 8900,
      summary: {
        user_requested: "Asked for a bare-bones app where a trip is a page with a title and a blank canvas underneath, no auth or database yet — just something to click around in.",
        ai_answered: "Proposed a Next.js app with trips stored in local component state for now, and a single /trip/[id] route so the URL shape wouldn't need to change later.",
        ai_implemented: "Generated the app shell, a trip creation modal, and an empty trip page with a placeholder \"drop your first idea here\" state.",
        meta: "First commit of the project. The user wanted the thinnest possible skeleton — a trip page that exists and can be created, nothing else. The assistant chose Next.js with app-router and kept all state in memory, explicitly deferring persistence and auth to avoid over-building before the core interaction (dropping ideas onto a board) was even sketched. It scaffolded routing, a create-trip modal, and an empty-state illustration slot."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants a minimal trip page — title plus empty canvas, no backend yet.", timestamp: "2026-06-18T09:04:10Z" },
        { kind: "AiWrote", summary: "Suggested Next.js with in-memory state and a /trip/[id] route shape to avoid future rework.", timestamp: "2026-06-18T09:05:02Z" },
        { kind: "AiChanged", summary: "Scaffolded the app, trip creation modal, and empty trip page.", timestamp: "2026-06-18T09:12:00Z" }
      ]
    },
    {
      hash: "9b1c2d3e4f506172839a4b5c6d7e8f90a1b2c3d4",
      date: "2026-06-18T14:03:00Z",
      title: "Trip creation flow",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 188, linesDeleted: 21, filesChanged: 6, minutesSincePrompt: 11, tokensIn: 59000, tokensOut: 5600,
      summary: {
        user_requested: "Wanted the create-trip modal to actually collect a destination and date range instead of just a title, and to redirect straight into the new trip.",
        ai_answered: "Explained it would keep dates optional since early testers might not know exact dates yet, defaulting to a rough season label instead.",
        ai_implemented: "Extended the modal with destination and optional date-range fields, added a lightweight in-memory trip store, and wired the redirect after creation.",
        meta: "The trip creation flow went from a title-only stub to something closer to real: destination, an optional date range, and immediate redirect into the new board. The assistant made dates optional on purpose, reasoning that a lot of trips start before dates are locked in, and added a fallback \"sometime this fall\" style label. Everything still lived in memory, which the assistant flagged as fine for now but not survivable across a refresh."
      },
      messages: [
        { kind: "UserWrote", summary: "Asked for destination + date range fields and a redirect into the new trip.", timestamp: "2026-06-18T13:50:44Z" },
        { kind: "AiChanged", summary: "Added the fields, in-memory store, and redirect.", timestamp: "2026-06-18T14:03:00Z" }
      ]
    },
    {
      hash: "1a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d",
      date: "2026-06-19T10:20:00Z",
      title: "Paste a link, get a card",
      agent: { provider: "Codex", model: "gpt-5.1-codex" },
      linesAdded: 340, linesDeleted: 12, filesChanged: 9, minutesSincePrompt: 24, tokensIn: 100500, tokensOut: 8600,
      summary: {
        user_requested: "The core idea: paste a URL and have it turn into a card with a title, photo, and source, rather than sitting there as raw text.",
        ai_answered: "Recommended a lightweight open-graph scraper on a serverless function so link previews wouldn't require a third-party API key yet.",
        ai_implemented: "Built a paste-to-card input, an /api/preview route that fetches and parses OG tags, and a card component with graceful fallbacks when no preview is available.",
        meta: "This is the feature the whole product hinges on. The user wanted pasting a link to feel instant and a little magical. The assistant built a serverless open-graph scraper rather than reaching for a paid API, keeping costs at zero during early testing, and made sure a failed scrape still produces a usable plain-link card instead of an error. It noted the scraper won't handle sites that require JavaScript to render their previews, which is a known gap for later."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants pasted links to turn into preview cards automatically.", timestamp: "2026-06-19T09:56:30Z" },
        { kind: "AiWrote", summary: "Proposed a self-hosted OG scraper instead of a paid preview API.", timestamp: "2026-06-19T09:58:12Z" },
        { kind: "AiChanged", summary: "Built the scraper endpoint and the link-to-card UI.", timestamp: "2026-06-19T10:20:00Z" }
      ]
    },
    {
      hash: "5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7081",
      date: "2026-06-19T21:47:00Z",
      title: "Day columns and drag-to-sort",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 455, linesDeleted: 38, filesChanged: 11, minutesSincePrompt: 33, tokensIn: 131000, tokensOut: 10800,
      summary: {
        user_requested: "Asked for the loose pile of cards to be sortable into day columns by dragging, similar to a kanban board but organized by date instead of status.",
        ai_answered: "Walked through using a drag-and-drop library rather than hand-rolling one, and suggested an \"unsorted\" column that always stays first so nothing gets lost.",
        ai_implemented: "Added day columns generated from the trip's date range plus a permanent Unsorted column, wired up drag-and-drop between them, and persisted order in memory.",
        meta: "The board went from a flat pile to a real itinerary shape: one column per day of the trip, with a permanent \"Unsorted\" column at the start so nothing gets lost before it has a home. The assistant chose an existing drag-and-drop library over a custom implementation to get keyboard accessibility and touch support for free, and it called out that reordering only persists in memory for now — a refresh still loses the layout, which the user would hit two days later."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants cards sortable into day columns via drag and drop.", timestamp: "2026-06-19T21:14:19Z" },
        { kind: "AiWrote", summary: "Suggested a drag-and-drop library and an always-present Unsorted column.", timestamp: "2026-06-19T21:16:40Z" },
        { kind: "AiChanged", summary: "Implemented day columns, Unsorted column, and drag-and-drop.", timestamp: "2026-06-19T21:47:00Z" }
      ]
    },
    {
      hash: "708192a3b4c5d6e7f8091a2b3c4d5e6f7081920",
      date: "2026-06-20T16:05:00Z",
      title: "Stop losing everything on refresh",
      agent: { provider: "Gemini", model: "gemini-2.5-pro" },
      linesAdded: 96, linesDeleted: 40, filesChanged: 5, minutesSincePrompt: 4, tokensIn: 30000, tokensOut: 3200,
      summary: {
        user_requested: "Reported losing an entire trip board on an accidental refresh and asked for anything that at least survives a reload.",
        ai_answered: "Recommended localStorage as a stopgap before a real backend, since the whole point right now is testing the drag-and-drop feel, not building accounts.",
        ai_implemented: "Serialized the trip and card state to localStorage on every change and rehydrated it on load, keyed by trip id.",
        meta: "A quick, reactive fix after the user lost a board they'd spent twenty minutes arranging. Rather than jumping straight to a database and auth, the assistant reached for localStorage keyed by trip id as an interim measure, reasoning that the immediate goal was to stop punishing testers for refreshing, not to solve multi-device sync — that would come later once collaboration was actually needed."
      },
      messages: [
        { kind: "UserWrote", summary: "Lost a whole trip board to an accidental refresh, wants it to survive reloads.", timestamp: "2026-06-20T16:01:05Z" },
        { kind: "AiChanged", summary: "Added localStorage persistence keyed by trip id.", timestamp: "2026-06-20T16:05:00Z" }
      ]
    },
    {
      hash: "8091a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3",
      date: "2026-06-22T11:30:00Z",
      title: "Invite people with a link",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 210, linesDeleted: 9, filesChanged: 7, minutesSincePrompt: 18, tokensIn: 76500, tokensOut: 6600,
      summary: {
        user_requested: "Asked for a way to send a trip to friends without them needing an account — a shareable link that drops them straight onto the board.",
        ai_answered: "Suggested a random unguessable trip-share token in the URL rather than the raw trip id, so boards aren't discoverable by guessing.",
        ai_implemented: "Added a share-link generator, a public /join/[token] route, and a banner prompting new visitors to pick a display name before editing.",
        meta: "The product's first step toward being a shared, not solo, tool. The user wanted zero-friction invites — no signup wall for someone who just wants to add \"that one restaurant.\" The assistant added an unguessable share token distinct from the trip's internal id so links couldn't be brute-forced, plus a lightweight name prompt for new visitors so cards could show who added what, without requiring a full account yet."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants friends to join a trip via link with no account required.", timestamp: "2026-06-22T11:12:50Z" },
        { kind: "AiWrote", summary: "Proposed an unguessable share token separate from the trip id.", timestamp: "2026-06-22T11:14:22Z" },
        { kind: "AiChanged", summary: "Built the share link, join route, and name prompt.", timestamp: "2026-06-22T11:30:00Z" }
      ]
    },
    {
      hash: "91a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4",
      date: "2026-06-22T19:58:00Z",
      title: "Magic-link sign-in",
      agent: { provider: "Codex", model: "gpt-5.1-codex" },
      linesAdded: 302, linesDeleted: 15, filesChanged: 10, minutesSincePrompt: 27, tokensIn: 106500, tokensOut: 8600,
      summary: {
        user_requested: "Wanted real accounts now that people were joining trips, but insisted on no passwords — email a link, click it, done.",
        ai_answered: "Laid out a magic-link flow using short-lived signed tokens emailed via a transactional mail API, with sessions stored in an httpOnly cookie.",
        ai_implemented: "Set up the mail-sending endpoint, token verification route, session cookie handling, and merged the name-prompt flow into real user records.",
        meta: "The transition from anonymous name-tags to real accounts, kept deliberately frictionless. The assistant implemented email-based magic links with short expiry windows rather than passwords, and folded the earlier display-name prompt into proper user records so existing trip contributions carried over. It noted that a returning user on a new device still needs to request a fresh link each time, which is expected but worth surfacing in onboarding."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants real passwordless accounts via magic email link.", timestamp: "2026-06-22T19:31:19Z" },
        { kind: "AiWrote", summary: "Outlined signed short-lived tokens plus httpOnly session cookies.", timestamp: "2026-06-22T19:33:40Z" },
        { kind: "AiChanged", summary: "Implemented magic-link auth end to end.", timestamp: "2026-06-22T19:58:00Z" }
      ]
    },
    {
      hash: "a2b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5",
      date: "2026-06-24T08:44:00Z",
      title: "Real-time board sync, take one",
      agent: { provider: "Claude Code", model: "claude-sonnet-4-6" },
      linesAdded: 275, linesDeleted: 30, filesChanged: 8, minutesSincePrompt: 41, tokensIn: 124500, tokensOut: 9800,
      summary: {
        user_requested: "Two people editing the same board were overwriting each other's changes; asked for edits to show up live for everyone on the trip.",
        ai_answered: "Proposed a WebSocket channel per trip that broadcasts card moves to connected clients, acknowledging edge cases around simultaneous drags would need follow-up.",
        ai_implemented: "Added a WebSocket server, a per-trip room abstraction, and client-side listeners that apply incoming moves to the board in real time.",
        meta: "First pass at real-time collaboration after the user watched two friends silently clobber each other's card placements. The assistant stood up a WebSocket room per trip broadcasting move events, deliberately scoping this pass to \"last write wins\" semantics and flagging that true conflict resolution for simultaneous drags of the same card was still unsolved — a gap the user would run into the very next day."
      },
      messages: [
        { kind: "UserWrote", summary: "Two collaborators are overwriting each other's edits; wants live sync.", timestamp: "2026-06-24T08:03:12Z" },
        { kind: "AiWrote", summary: "Proposed a per-trip WebSocket room broadcasting moves, last-write-wins for now.", timestamp: "2026-06-24T08:05:50Z" },
        { kind: "AiChanged", summary: "Implemented the WebSocket sync layer.", timestamp: "2026-06-24T08:44:00Z" }
      ]
    },
    {
      hash: "b3c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6",
      date: "2026-06-25T13:12:00Z",
      title: "Fix the sync race condition",
      agent: { provider: "Claude Code", model: "claude-sonnet-4-6" },
      linesAdded: 88, linesDeleted: 52, filesChanged: 4, minutesSincePrompt: 15, tokensIn: 48500, tokensOut: 4600,
      summary: {
        user_requested: "Reported cards occasionally snapping back to their old position when two people dragged around the same time, and wanted it stable before inviting more testers.",
        ai_answered: "Diagnosed a stale-state overwrite where a slow client's broadcast arrived after a faster one and clobbered it, and proposed attaching a version counter to each move.",
        ai_implemented: "Added a monotonic version number per card move and had clients discard incoming updates older than their current version.",
        meta: "A focused bug-fix session addressing the flakiness introduced by the previous day's real-time sync work. The assistant traced the snap-back behavior to out-of-order network delivery — a slow client's stale broadcast landing after a newer one — and fixed it with a simple version counter per card rather than a heavier operational-transform approach, judging that overkill for a board where whole-card moves, not concurrent text edits, are what's being synced."
      },
      messages: [
        { kind: "UserWrote", summary: "Cards randomly snap back during simultaneous drags; wants it fixed before more testers join.", timestamp: "2026-06-25T12:57:03Z" },
        { kind: "AiWrote", summary: "Diagnosed an out-of-order broadcast race and proposed a version counter fix.", timestamp: "2026-06-25T12:58:44Z" },
        { kind: "AiChanged", summary: "Added version counters to discard stale moves.", timestamp: "2026-06-25T13:12:00Z" }
      ]
    },
    {
      hash: "c4d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f7",
      date: "2026-06-27T20:31:00Z",
      title: "Give it a face",
      agent: { provider: "Gemini", model: "gemini-2.5-pro" },
      linesAdded: 402, linesDeleted: 61, filesChanged: 16, minutesSincePrompt: 52, tokensIn: 182000, tokensOut: 13200,
      summary: {
        user_requested: "Asked for a visual pass — real empty states, a proper icon set, consistent spacing — since the app had been function-first and looked unfinished to new testers.",
        ai_answered: "Suggested a small consistent icon set and a single accent color to unify the previously ad-hoc button and card styling.",
        ai_implemented: "Replaced placeholder text empty-states with illustrated ones, added a cohesive icon set, and normalized spacing and color tokens across components.",
        meta: "A pure polish pass with no new functionality, prompted by feedback that the app \"worked but looked like a prototype.\" The assistant introduced a small shared icon set and a single accent color used consistently across buttons and highlights, replacing what had become an inconsistent grab-bag of ad-hoc styles from earlier feature-focused commits, and gave every empty state (no trip yet, empty day column, no photos) real illustrated treatment instead of plain text."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants a visual polish pass — icons, empty states, consistent spacing.", timestamp: "2026-06-27T19:39:10Z" },
        { kind: "AiWrote", summary: "Proposed a shared icon set and single accent color to unify styling.", timestamp: "2026-06-27T19:41:55Z" },
        { kind: "AiChanged", summary: "Applied the visual polish across components.", timestamp: "2026-06-27T20:31:00Z" }
      ]
    },
    {
      hash: "d5e6f708192a3b4c5d6e7f8091a2b3c4d5e6f708",
      date: "2026-06-29T09:05:00Z",
      title: "Photos on cards, not just links",
      agent: { provider: "Codex", model: "gpt-5.1-codex" },
      linesAdded: 264, linesDeleted: 18, filesChanged: 9, minutesSincePrompt: 20, tokensIn: 88000, tokensOut: 7400,
      summary: {
        user_requested: "Wanted to attach an actual photo to a card directly — a screenshot of a hotel listing, a picture from a friend's last trip — not just a scraped link preview.",
        ai_answered: "Recommended direct-to-storage uploads from the browser so large images wouldn't route through the app server, with client-side resizing before upload.",
        ai_implemented: "Added drag-and-drop photo upload onto any card, client-side image resizing, and a storage bucket with per-trip access rules.",
        meta: "Extended cards beyond link previews to accept photos directly, since a lot of trip inspiration starts as a screenshot rather than a URL. The assistant chose direct-to-storage browser uploads with pre-upload resizing to keep the app server lightweight and keep large images from ballooning page load times, and scoped storage access rules to the trip's collaborators only."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants to attach photos directly to cards, not just link previews.", timestamp: "2026-06-29T08:45:31Z" },
        { kind: "AiChanged", summary: "Built direct-to-storage photo uploads with client-side resizing.", timestamp: "2026-06-29T09:05:00Z" }
      ]
    },
    {
      hash: "e6f708192a3b4c5d6e7f8091a2b3c4d5e6f70819",
      date: "2026-06-29T22:40:00Z",
      title: "A map, not just a list",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 318, linesDeleted: 22, filesChanged: 7, minutesSincePrompt: 35, tokensIn: 113500, tokensOut: 9600,
      summary: {
        user_requested: "Asked for a map showing every pinned place across the trip, since the day-by-day list didn't make obvious how much walking a day would involve.",
        ai_answered: "Suggested geocoding each card's location once on save and caching the coordinates, rather than re-geocoding on every board view.",
        ai_implemented: "Added a map tab that plots every card with a location, color-coded by day, and a lightweight geocoding step that runs when a card is created or edited.",
        meta: "A second view of the same trip data, aimed at answering \"are we overplanning this day?\" at a glance. The assistant geocoded locations once at card-save time and cached the result rather than hitting the geocoding API on every board load, and color-coded pins by day so the spatial spread of a day's plan is visible without opening the list."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants a map view of all pinned places, color-coded by day.", timestamp: "2026-06-29T22:05:12Z" },
        { kind: "AiWrote", summary: "Proposed geocoding once at save-time and caching coordinates.", timestamp: "2026-06-29T22:07:30Z" },
        { kind: "AiChanged", summary: "Built the map tab with cached geocoding.", timestamp: "2026-06-29T22:40:00Z" }
      ]
    },
    {
      hash: "f708192a3b4c5d6e7f8091a2b3c4d5e6f708192a",
      date: "2026-07-01T12:15:00Z",
      title: "Fix drag-reorder on iOS Safari",
      agent: { provider: "Claude Code", model: "claude-sonnet-4-6" },
      linesAdded: 41, linesDeleted: 29, filesChanged: 3, minutesSincePrompt: 9, tokensIn: 30500, tokensOut: 3200,
      summary: {
        user_requested: "A tester on an iPhone reported that dragging a card sometimes deleted it instead of moving it, only happening on Safari.",
        ai_answered: "Identified a touch-event handling difference in Safari where a long-press drag was being misread as a swipe-to-dismiss gesture.",
        ai_implemented: "Adjusted the drag library's touch thresholds and disabled a conflicting swipe gesture on card elements specifically for iOS Safari.",
        meta: "A targeted mobile bug fix reported by an actual tester rather than found in dev. The assistant traced the card-deletion behavior to Safari's touch-event quirks conflicting with the drag library's long-press detection, and scoped the fix to iOS Safari specifically rather than changing global drag behavior, to avoid regressing the desktop and Android experience that was already working."
      },
      messages: [
        { kind: "UserWrote", summary: "Reports cards getting deleted instead of moved when dragged on iPhone Safari.", timestamp: "2026-07-01T12:06:40Z" },
        { kind: "AiWrote", summary: "Traced it to a Safari touch-event conflict with swipe-to-dismiss.", timestamp: "2026-07-01T12:08:02Z" },
        { kind: "AiChanged", summary: "Fixed touch thresholds scoped to iOS Safari.", timestamp: "2026-07-01T12:15:00Z" }
      ]
    },
    {
      hash: "08192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b",
      date: "2026-07-02T17:50:00Z",
      title: "Make it work on a plane",
      agent: { provider: "Gemini", model: "gemini-2.5-pro" },
      linesAdded: 233, linesDeleted: 19, filesChanged: 6, minutesSincePrompt: 47, tokensIn: 132000, tokensOut: 10500,
      summary: {
        user_requested: "Asked for the board to stay usable with no signal, since people tend to plan trips on flights and in transit.",
        ai_answered: "Proposed a service worker caching the current trip's data and queuing edits made offline, replaying them once connectivity returns.",
        ai_implemented: "Added a service worker with an offline cache of the active trip, a local edit queue, and a sync-on-reconnect handler.",
        meta: "Offline support aimed squarely at the moment people actually plan trips — on a flight or the subway with no signal. The assistant added a service worker that caches the currently open trip and queues any edits made while offline, replaying them against the server once connectivity returns, and flagged that conflicting edits made offline by two people simultaneously still fall back to last-write-wins."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants the board usable offline, e.g. on a flight, with edits synced later.", timestamp: "2026-07-02T17:03:15Z" },
        { kind: "AiWrote", summary: "Proposed a service worker with an offline cache and edit queue.", timestamp: "2026-07-02T17:05:48Z" },
        { kind: "AiChanged", summary: "Implemented offline caching and sync-on-reconnect.", timestamp: "2026-07-02T17:50:00Z" }
      ]
    },
    {
      hash: "192a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c",
      date: "2026-07-04T10:00:00Z",
      title: "A tour for first-time visitors",
      agent: { provider: "Codex", model: "gpt-5.1-codex" },
      linesAdded: 176, linesDeleted: 8, filesChanged: 5, minutesSincePrompt: 22, tokensIn: 74500, tokensOut: 6600,
      summary: {
        user_requested: "New collaborators joining via a share link had no idea what to do first; asked for a short guided tour on first visit.",
        ai_answered: "Suggested a four-step overlay tour pointing at the paste box, day columns, map tab, and invite button, dismissible and never shown twice.",
        ai_implemented: "Built a lightweight onboarding overlay triggered on a visitor's first join, storing dismissal state per user so it never reappears.",
        meta: "Addressed confusion from new joiners who landed on a board mid-trip-planning with no context. The assistant built a short four-stop overlay tour highlighting the paste box, the day columns, the map tab, and the invite button, storing a per-user dismissal flag so returning visitors never see it again, and kept it skippable at any point rather than forcing all four steps."
      },
      messages: [
        { kind: "UserWrote", summary: "New joiners are confused on first visit; wants a short guided tour.", timestamp: "2026-07-04T09:38:20Z" },
        { kind: "AiChanged", summary: "Built the dismissible four-step onboarding tour.", timestamp: "2026-07-04T10:00:00Z" }
      ]
    },
    {
      hash: "2a3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d",
      date: "2026-07-05T21:22:00Z",
      title: "Speed up big trips",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 149, linesDeleted: 66, filesChanged: 6, minutesSincePrompt: 13, tokensIn: 58500, tokensOut: 5400,
      summary: {
        user_requested: "A three-week trip with 200+ cards had gotten noticeably janky to scroll and drag; asked for it to feel smooth again regardless of trip size.",
        ai_answered: "Diagnosed the whole board rendering every card's DOM at once regardless of scroll position, and proposed windowed rendering per column.",
        ai_implemented: "Added virtualized rendering so only cards near the visible viewport mount, cutting initial render and drag latency on large boards.",
        meta: "A performance pass triggered by a real large trip stress-testing the board. The assistant found that every card across every day column was mounting to the DOM regardless of what was actually visible, and fixed it with per-column virtualization so only nearby cards render, which noticeably improved both scroll smoothness and drag responsiveness on the 200-card test trip without changing any behavior."
      },
      messages: [
        { kind: "UserWrote", summary: "A 200+ card trip is janky to scroll and drag; wants it smooth at any size.", timestamp: "2026-07-05T21:09:04Z" },
        { kind: "AiWrote", summary: "Diagnosed unbounded DOM mounting and proposed per-column virtualization.", timestamp: "2026-07-05T21:11:30Z" },
        { kind: "AiChanged", summary: "Implemented virtualized card rendering.", timestamp: "2026-07-05T21:22:00Z" }
      ]
    },
    {
      hash: "3b4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e",
      date: "2026-07-07T15:33:00Z",
      title: "A public page for the finished plan",
      agent: { provider: "Claude Code", model: "claude-opus-4-6" },
      linesAdded: 289, linesDeleted: 14, filesChanged: 8, minutesSincePrompt: 30, tokensIn: 102500, tokensOut: 8400,
      summary: {
        user_requested: "Wanted a clean, read-only page to send to non-collaborators — parents, a group chat — without giving them edit access or requiring an account.",
        ai_answered: "Proposed a separate public route rendering a read-only snapshot, refreshed periodically rather than live, to keep it simple and fast.",
        ai_implemented: "Added a public /trip/[token]/view page with a static, print-friendly rendering of the itinerary, plus a toggle to enable or disable public access.",
        meta: "The feature that turns a private planning board into something shareable with the wider world, not just collaborators. The assistant deliberately made the public view a periodically-refreshed snapshot rather than a live connection, judging that a group chat sharing a link doesn't need real-time updates, and added an explicit per-trip toggle so owners opt in rather than every trip being public by default."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants a read-only public page to share with non-collaborators.", timestamp: "2026-07-07T15:03:11Z" },
        { kind: "AiWrote", summary: "Proposed a periodically-refreshed public snapshot with an opt-in toggle.", timestamp: "2026-07-07T15:05:40Z" },
        { kind: "AiChanged", summary: "Built the public read-only view and access toggle.", timestamp: "2026-07-07T15:33:00Z" }
      ]
    },
    {
      hash: "4c5d6e7f8091a2b3c4d5e6f708192a3b4c5d6e7f",
      date: "2026-07-08T23:10:00Z",
      title: "Launch polish",
      agent: { provider: "Claude Code", model: "claude-sonnet-4-6" },
      linesAdded: 122, linesDeleted: 33, filesChanged: 11, minutesSincePrompt: 19, tokensIn: 84000, tokensOut: 6100,
      summary: {
        user_requested: "Final pre-launch pass: real favicon and app icon, social preview meta tags, and basic analytics before sharing the link publicly.",
        ai_answered: "Recommended minimal, privacy-respecting analytics (page views and core actions only, no third-party trackers) to keep the launch lightweight.",
        ai_implemented: "Added the app icon set, Open Graph and Twitter card meta tags for link previews, and a small first-party analytics script tracking key actions.",
        meta: "The last commit before sharing Driftlist publicly. Housekeeping rather than features: a proper icon set replacing the default placeholder, social preview meta tags so shared links look intentional, and a small first-party analytics setup tracking only page views and a handful of key actions like trip creation and invites — the assistant specifically avoided third-party trackers to keep the launch lightweight and privacy-respecting."
      },
      messages: [
        { kind: "UserWrote", summary: "Wants favicon, app icon, social preview tags, and basic analytics before launch.", timestamp: "2026-07-08T22:51:37Z" },
        { kind: "AiWrote", summary: "Recommended minimal first-party analytics over third-party trackers.", timestamp: "2026-07-08T22:53:20Z" },
        { kind: "AiChanged", summary: "Added icons, social meta tags, and first-party analytics.", timestamp: "2026-07-08T23:10:00Z" }
      ]
    }
  ]
};
