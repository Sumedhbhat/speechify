import { describe, expect, it } from "vitest";
import type { Span } from "../src/api";
import { modelLine, stepsLine, wait, waitLine } from "../src/timing";
import { dictation } from "./fakes";

const trace: Span[] = [
  { stage: "piece", at_ms: -4000, ms: 600, chunk: 0, audio_ms: 3000, words: 40 },
  { stage: "model", at_ms: -900, ms: 1400, chunk: 0, words: 40, model: { prompt_ms: 1200 } },
  { stage: "stop", at_ms: 0, ms: 12 },
  { stage: "load", at_ms: 12, ms: 0 },
  { stage: "speech", at_ms: 12, ms: 300, chunk: 0 },
  { stage: "piece", at_ms: 20, ms: 280, chunk: 1, words: 6 },
  { stage: "settle", at_ms: 312, ms: 188 },
  {
    stage: "model",
    at_ms: 500,
    ms: 450,
    chunk: 1,
    words: 6,
    model: { load_ms: 3, prompt_ms: 380, prompt_tokens: 612, answer_ms: 60, answer_tokens: 9 },
  },
  { stage: "cleanup", at_ms: 500, ms: 460 },
  { stage: "paste", at_ms: 960, ms: 45 },
];

describe("timing", () => {
  it("splits the wait after stopping into speech, clean-up and the rest", () => {
    const item = dictation({ trace });
    expect(wait(item)).toEqual({ speech: 300, cleanup: 648, other: 57, total: 1005 });
    expect(waitLine(item)).toBe("0.30 s + 0.65 s + 0.06 s = 1.01 s");
  });

  it("lists each step after stopping and what the model spent its time on", () => {
    expect(stepsLine(dictation({ trace }))).toBe(
      "stop mic 12 ms → speech 300 ms → waiting on clean-up 188 ms → clean-up 460 ms → paste 45 ms",
    );
    expect(modelLine(dictation({ trace }))).toBe(
      "prompt 1,200 ms · load 3 ms, prompt 380 ms / 612 tokens, answer 60 ms / 9 tokens",
    );
  });

  it("falls back to the old totals for dictations made before tracing", () => {
    const old = dictation({ asr_ms: 320, rewrite_ms: 210, total_ms: 600, trace: [] });
    expect(waitLine(old)).toBe("0.32 s + 0.21 s + 0.07 s = 0.60 s");
    expect(stepsLine(old)).toBeNull();
    expect(modelLine(old)).toBeNull();
    expect(wait(dictation({ asr_ms: 500, rewrite_ms: 200, total_ms: 600 })).other).toBe(0);
  });

  it("shows tokens alone when the server reports no times", () => {
    const tokensOnly: Span[] = [
      { stage: "cleanup", at_ms: 0, ms: 100 },
      { stage: "model", at_ms: 0, ms: 90, model: { prompt_tokens: 90, answer_tokens: 7 } },
      { stage: "model", at_ms: 0, ms: 90, model: {} },
      { stage: "model", at_ms: 0, ms: 90 },
    ];
    expect(stepsLine(dictation({ trace: tokensOnly }))).toBe("clean-up 100 ms");
    expect(modelLine(dictation({ trace: tokensOnly }))).toBe("prompt 90 tokens, answer 7 tokens");
  });
});
