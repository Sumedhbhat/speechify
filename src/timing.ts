import type { Dictation, Span, Stage } from "./api";
import { formatNumber } from "./format";

export interface Wait {
  speech: number;
  cleanup: number;
  other: number;
  total: number;
}

const STEPS: Partial<Record<Stage, string>> = {
  stop: "stop mic",
  load: "models",
  speech: "speech",
  settle: "waiting on clean-up",
  cleanup: "clean-up",
  paste: "paste",
};

function steps(item: Dictation): Span[] {
  return item.trace.filter((span) => span.stage in STEPS && span.at_ms >= 0);
}

function sum(spans: Span[], ...stages: Stage[]): number {
  return spans
    .filter((span) => stages.includes(span.stage))
    .reduce((all, span) => all + span.ms, 0);
}

export function wait(item: Dictation): Wait {
  const after = steps(item);
  if (after.length === 0) {
    const other = Math.max(0, item.total_ms - item.asr_ms - item.rewrite_ms);
    return { speech: item.asr_ms, cleanup: item.rewrite_ms, other, total: item.total_ms };
  }
  const total = Math.max(...after.map((span) => span.at_ms + span.ms));
  const speech = sum(after, "speech");
  const cleanup = sum(after, "settle", "cleanup");
  return { speech, cleanup, other: Math.max(0, total - speech - cleanup), total };
}

export function seconds(ms: number): string {
  return `${(Math.round(ms / 10) / 100).toFixed(2)} s`;
}

function millis(ms: number): string {
  return `${formatNumber(ms)} ms`;
}

export function waitLine(item: Dictation): string {
  const { speech, cleanup, other, total } = wait(item);
  return `${seconds(speech)} + ${seconds(cleanup)} + ${seconds(other)} = ${seconds(total)}`;
}

function withTokens(ms: number | undefined, tokens: number | undefined): string | null {
  if (ms === undefined && tokens === undefined) return null;
  const time = ms === undefined ? "" : millis(ms);
  const count = tokens === undefined ? "" : `${formatNumber(tokens)} tokens`;
  return time && count ? `${time} / ${count}` : time || count;
}

function modelPart(span: Span): string | null {
  const model = span.model;
  if (!model) return null;
  const prompt = withTokens(model.prompt_ms, model.prompt_tokens);
  const answer = withTokens(model.answer_ms, model.answer_tokens);
  const parts = [
    model.load_ms ? `load ${millis(model.load_ms)}` : null,
    prompt && `prompt ${prompt}`,
    answer && `answer ${answer}`,
  ].filter((part): part is string => Boolean(part));
  return parts.length ? parts.join(", ") : null;
}

export function stepsLine(item: Dictation): string | null {
  const after = steps(item);
  if (after.length === 0) return null;
  return after
    .filter((span) => span.ms > 0 || span.stage === "speech" || span.stage === "cleanup")
    .map((span) => `${STEPS[span.stage]} ${millis(span.ms)}`)
    .join(" → ");
}

export function modelLine(item: Dictation): string | null {
  const waitedOn = item.trace
    .filter((span) => span.stage === "model" && span.at_ms + span.ms > 0)
    .map(modelPart)
    .filter((part): part is string => part !== null);
  return waitedOn.length ? waitedOn.join(" · ") : null;
}
