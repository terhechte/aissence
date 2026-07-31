model choice:

inclusionai/ling-2.6-flash	$0.00090526.	Fast		short	LIKE!

## Model benchmarks

Run the complete summarization pipeline for one model and source project:

```sh
cargo run --bin aissence-benchmark -- \
  --model inclusionai/ling-2.6-flash \
  --source-folder /path/to/project
```

Set `OPENROUTER_API_KEY` first. `OPENROUTER_PROVIDER` remains optional.

In an interactive terminal, the benchmark shows a live request dashboard with
in-flight request types, elapsed time, streaming output-token estimates, retry
state, and recent errors. Estimates are replaced with exact provider counts
when a request completes. Set `NO_COLOR=1` to disable the animation and ANSI
color sequences.

The command performs a fresh run and, by default, skips the redundant
`User requested`, `AI implemented`, and `Overview` summaries. Pass
`--meta-summaries` to include those extended summaries. It writes both files to
the current directory rather than choosing the source folder:

- `inclusionai-ling-2.6-flash.aissence` — the model-specific summary cache
- `inclusionai-ling-2.6-flash-stats.md` — elapsed time, reported OpenRouter
  cost, request counts, and the first/last prompt and aggregate summaries

Characters such as `/` in an OpenRouter model ID become `-` in filenames. The
full model ID is recorded unchanged in the stats report.
