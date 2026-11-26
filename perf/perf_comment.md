## Performance for `$commit`

$comparison
Timings come from shared GitHub runners: changes within ±$noise are usually noise, so only larger ones are marked (⚠️ worse, 🟢 better).
$missing

### 1. Transcription

Speech to raw text, for one recording of $seconds s.

$transcription

### 2. Cleanup

Raw text to the text that gets typed.

$cleanup

### 3. Scenarios

Scenarios passed in each group, by the rules alone and with the language model.

$scenarios
$changes

### Verdict

$verdict
$failures
$json

$models
Scenarios: `perf/scenarios.json`. Cleanup prompt: `core/src/prompts/cleanup.txt`. Each merge to `main` that changes the engine is measured again; every nightly release carries the latest results as `performance.json`.
$run
