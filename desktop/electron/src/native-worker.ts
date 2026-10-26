// This process owns native pointers. Terminating it cancels a blocked native call.
import koffi from "koffi";
import path from "node:path";
import { splitAudio } from "./core";
const dir = process.env.SPEECHIFY_NATIVE!;
const lib = koffi.load(
  path.join(
    dir,
    process.platform === "win32"
      ? "transcribe.dll"
      : process.platform === "darwin"
        ? "libtranscribe.dylib"
        : "libtranscribe.so",
  ),
);
const fn = (name: string, ret: string, args: any[]) =>
  lib.func(name, ret, args);
const init = fn("transcribe_init_backends", "int", ["str"]);
const loadParams = koffi.struct("LoadParams", {
  struct_size: "uint64",
  backend: "int",
  device: "void *",
});
const sessionParams = koffi.struct("SessionParams", {
  struct_size: "uint64",
  n_threads: "int",
  kv_type: "int",
  n_ctx: "int32",
});
const runParams = koffi.struct("RunParams", {
  struct_size: "uint64",
  task: "int",
  timestamps: "int",
  pnc: "int",
  itn: "int",
  diarize: "int",
  language: "str",
  target_language: "str",
  keep_special_tags: "bool",
  family: "void *",
  spec_k_drafts: "int32",
  vocabulary: "void *",
  n_vocabulary: "int32",
  prompt: "str",
  prefix: "str",
});
const tokenType = koffi.struct("Token", {
  struct_size: "uint64",
  id: "int",
  p: "float",
  t0_ms: "int64",
  t1_ms: "int64",
  seg_index: "int",
  word_index: "int",
  text: "str",
});
const initLoad = fn("transcribe_model_load_params_init", "void", [
  koffi.out(koffi.pointer(loadParams)),
]);
const initSession = fn("transcribe_session_params_init", "void", [
  koffi.out(koffi.pointer(sessionParams)),
]);
const initRun = fn("transcribe_run_params_init", "void", [
  koffi.out(koffi.pointer(runParams)),
]);
const load = fn("transcribe_model_load_file", "int", [
  "str",
  koffi.pointer(loadParams),
  koffi.out(koffi.pointer("void *")),
]);
const sessionInit = fn("transcribe_session_init", "int", [
  "void *",
  koffi.pointer(sessionParams),
  koffi.out(koffi.pointer("void *")),
]);
const run = fn("transcribe_run", "int", [
  "void *",
  "float *",
  "int",
  koffi.pointer(runParams),
]);
const fullText = fn("transcribe_full_text", "str", ["void *"]);
const backend = fn("transcribe_model_backend", "str", ["void *"]);
const status = fn("transcribe_status_string", "str", ["int"]);
const supports = fn("transcribe_model_supports", "bool", ["void *", "int"]);
const freeSession = fn("transcribe_session_free", "void", ["void *"]);
const freeModel = fn("transcribe_model_free", "void", ["void *"]);
const tokenInit = fn("transcribe_token_init", "void", [
  koffi.out(koffi.pointer(tokenType)),
]);
const tokenGet = fn("transcribe_get_token", "int", [
  "void *",
  "int",
  koffi.inout(koffi.pointer(tokenType)),
]);
const countTokens = fn("transcribe_n_tokens", "int", ["void *"]);
const capsType = koffi.struct("Capabilities", {
  struct_size: "uint64",
  native_sample_rate: "int32",
  n_languages: "int",
  languages: "void *",
  max_timestamp_kind: "int",
  supports_language_detect: "bool",
  supports_translate: "bool",
  supports_streaming: "bool",
  supports_spec_decode: "bool",
  max_audio_ms: "int64",
  n_translate_target_languages: "int",
  translate_target_languages: "void *",
});
const limitsType = koffi.struct("Limits", {
  struct_size: "uint64",
  effective_n_ctx: "int32",
  effective_max_audio_ms: "int64",
  max_kv_bytes: "int64",
});
const capsInit = fn("transcribe_capabilities_init", "void", [
  koffi.out(koffi.pointer(capsType)),
]);
const capsGet = fn("transcribe_model_get_capabilities", "int", [
  "void *",
  koffi.inout(koffi.pointer(capsType)),
]);
const limitsInit = fn("transcribe_session_limits_init", "void", [
  koffi.out(koffi.pointer(limitsType)),
]);
const limitsGet = fn("transcribe_session_get_limits", "int", [
  "void *",
  koffi.inout(koffi.pointer(limitsType)),
]);
let maxSamples = 0,
  languages: string[] = [];
let model: any = null,
  session: any = null,
  options: any;
function check(code: number) {
  if (code !== 0) throw Error(status(code));
}
function close() {
  if (session) freeSession(session);
  if (model) freeModel(model);
  model = session = null;
}
function open(device: string) {
  close();
  const lp: any = {};
  initLoad(lp);
  lp.backend =
    ({ auto: 0, cpu: 1, metal: 2, vulkan: 3, cuda: 5 } as any)[device] ?? 1;
  const m: any[] = [null];
  check(load(options.file, lp, m));
  model = m[0];
  const sp: any = {};
  initSession(sp);
  sp.n_threads = options.threads;
  const s: any[] = [null];
  check(sessionInit(model, sp, s));
  session = s[0];
  const caps: any = {},
    limits: any = {};
  capsInit(caps);
  check(capsGet(model, caps));
  limitsInit(limits);
  check(limitsGet(session, limits));
  const ceilings = [
    Number(caps.max_audio_ms),
    Number(limits.effective_max_audio_ms),
  ].filter((n) => n > 0);
  maxSamples = ceilings.length ? Math.floor(Math.min(...ceilings) * 16) : 0;
  languages =
    caps.n_languages > 0
      ? (koffi.decode(caps.languages, "str", caps.n_languages) as string[])
      : [];
}
function transcribePiece(pcm: Float32Array, prompt: string) {
  const rp: any = {};
  initRun(rp);
  rp.language = languages.includes(options.language) ? options.language : null;
  if (supports(model, 4)) rp.pnc = 1;
  // Generic context is honored only on families advertising it. Whisper extension
  // is initialized into oversized aligned storage, so ABI defaults remain intact.
  let ext: Buffer | undefined, promptBuffer: Buffer | undefined;
  if (supports(model, 0)) {
    ext = Buffer.alloc(256);
    fn("transcribe_whisper_run_ext_init", "void", ["void *"])(ext);
    promptBuffer = Buffer.from(
      (
        (options.language === "en"
          ? "Hello. This is a clear, punctuated English sentence. "
          : "") + prompt
      ).slice(0, 600) + "\0",
    );
    koffi.encode(ext, 16, "void *", promptBuffer);
    rp.family = ext;
  } else if (supports(model, 8)) rp.prompt = prompt || null;
  let words: Buffer[] = [];
  let pointers: Buffer | undefined;
  if (supports(model, 7) && prompt) {
    words = prompt
      .split(",")
      .filter(Boolean)
      .slice(0, 80)
      .map((s) => Buffer.from(s.trim() + "\0"));
    pointers = Buffer.alloc(words.length * 8);
    words.forEach((w, i) => koffi.encode(pointers!, i * 8, "void *", w));
    rp.vocabulary = pointers;
    rp.n_vocabulary = words.length;
  }
  check(run(session, pcm, pcm.length, rp));
  let text = fullText(session).trim();
  const wordsInText = text.split(/\s+/);
  if (wordsInText.length >= 3) {
    if (wordsInText[0] === wordsInText[0].toLowerCase())
      text = text.charAt(0).toUpperCase() + text.slice(1);
    if (/[\p{L}\p{N}]$/u.test(text)) text += ".";
  }
  let sum = 0,
    n = 0;
  for (let i = 0; i < countTokens(session); i++) {
    const t: any = {};
    tokenInit(t);
    check(tokenGet(session, i, t));
    if (t.p > 0 && t.p <= 1) {
      sum += Math.log(t.p);
      n++;
    }
  }
  return {
    text,
    confidence: n ? Math.exp(sum / n) : null,
    device: backend(model),
  };
}
function transcribe(pcm: Float32Array, prompt: string) {
  const pieces = splitAudio(pcm, maxSamples);
  const results = pieces.map((piece) => transcribePiece(piece, prompt));
  const scored = results.filter((r) => r.confidence !== null);
  return {
    text: results
      .map((r) => r.text)
      .filter(Boolean)
      .join(" "),
    confidence: scored.length
      ? Math.exp(
          scored.reduce((n, r) => n + Math.log(r.confidence!), 0) /
            scored.length,
        )
      : null,
    device: backend(model),
  };
}
check(init(dir));
process.on("message", (msg: any) => {
  try {
    if (msg.type === "load") {
      options = msg;
      const start = performance.now();
      try {
        open(msg.device);
      } catch (e) {
        if (msg.device === "cpu") throw e;
        open("cpu");
      }
      process.send?.({
        id: msg.id,
        ready: true,
        device: backend(model),
        ms: performance.now() - start,
      });
    } else if (msg.type === "run") {
      const pcm = Float32Array.from(msg.pcm);
      const start = performance.now();
      let result;
      try {
        result = transcribe(pcm, msg.prompt);
      } catch (e) {
        if (backend(model).toLowerCase() === "cpu") throw e;
        open("cpu");
        result = transcribe(pcm, msg.prompt);
      }
      process.send?.({ id: msg.id, ...result, ms: performance.now() - start });
    } else if (msg.type === "close") {
      close();
      process.exit(0);
    }
  } catch (e) {
    process.send?.({ id: msg.id, error: String(e) });
  }
});
process.on("disconnect", () => {
  close();
  process.exit(0);
});
