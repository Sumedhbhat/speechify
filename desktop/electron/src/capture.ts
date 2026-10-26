// Audio transport only. Main owns when to record and what to do with the clip.
declare const capture: {
  on: (f: (s: any) => void) => void;
  send: (e: string, d: any) => void;
};
let recordingId = 0;
let stream: MediaStream | null = null,
  ctx: AudioContext | null = null,
  processor: ScriptProcessorNode | null = null,
  recording = false,
  keep = false,
  selected = "",
  chunks: Float32Array[] = [],
  pre: Float32Array[] = [],
  preCount = 0,
  total = 0,
  generation = 0,
  source: MediaStreamAudioSourceNode | null = null,
  serial = Promise.resolve();
async function devices() {
  const list = await navigator.mediaDevices.enumerateDevices();
  capture.send(
    "devices",
    list
      .filter((d) => d.kind === "audioinput")
      .map((d) => ({
        id: d.deviceId,
        label: d.label || "Microphone (permission needed)",
      })),
  );
}
async function closeMic() {
  stream?.getTracks().forEach((t) => t.stop());
  processor?.disconnect();
  source?.disconnect();
  await ctx?.close();
  stream = null;
  processor = null;
  source = null;
  ctx = null;
  pre = [];
  preCount = 0;
}
async function openMic() {
  if (stream) return;
  try {
    stream = await navigator.mediaDevices.getUserMedia({
      audio: {
        deviceId: selected ? { exact: selected } : undefined,
        channelCount: 1,
        echoCancellation: false,
        noiseSuppression: false,
        autoGainControl: false,
      },
      video: false,
    });
  } catch (e) {
    if (selected && (e as DOMException).name === "OverconstrainedError") {
      selected = "";
      capture.send("missing-device", null);
      stream = await navigator.mediaDevices.getUserMedia({ audio: true });
    } else throw e;
  }
  ctx = new AudioContext();
  source = ctx.createMediaStreamSource(stream);
  processor = ctx.createScriptProcessor(4096, 1, 1);
  source.connect(processor);
  processor.connect(ctx.destination);
  processor.onaudioprocess = (e) => {
    const a = e.inputBuffer.getChannelData(0).slice();
    let rms = 0;
    for (const n of a) rms += n * n;
    capture.send("level", Math.sqrt(rms / a.length));
    if (recording) {
      chunks.push(a);
      total += a.length;
      if (total > ctx!.sampleRate * 120) {
        recording = false;
        capture.send("error", "Recording limit is 120 seconds");
        chunks = [];
        if (!keep) void closeMic();
      }
    } else if (keep) {
      pre.push(a);
      preCount += a.length;
      while (
        pre.length > 1 &&
        preCount - pre[0].length >= ctx!.sampleRate * 0.3
      )
        preCount -= pre.shift()!.length;
    }
  };
  stream.getAudioTracks()[0].onended = () => {
    capture.send("error", "Microphone disconnected; choose an available input");
    recording = false;
    void closeMic();
  };
  await ctx.resume();
  capture.send("mic-ready", null);
  await devices();
}
async function end(accept: boolean) {
  if (!recording) {
    if (accept)
      capture.send(
        "error",
        "Microphone was not ready before release. Hold the talk key longer or enable pre-roll.",
      );
    if (!keep) await closeMic();
    return;
  }
  recording = false;
  if (accept && ctx && total > 0) {
    const rate = ctx.sampleRate;
    const all = new Float32Array(total);
    let p = 0;
    for (const a of chunks) {
      all.set(a, p);
      p += a.length;
    }
    const offline = new OfflineAudioContext(
      1,
      Math.max(1, Math.round((total * 16000) / rate)),
      16000,
    );
    const buffer = offline.createBuffer(1, total, rate);
    buffer.copyToChannel(all, 0);
    const s = offline.createBufferSource();
    s.buffer = buffer;
    s.connect(offline.destination);
    s.start();
    const rendered = await offline.startRendering();
    capture.send("audio", {
      id: recordingId,
      pcm: Array.from(rendered.getChannelData(0)),
    });
  }
  chunks = [];
  total = 0;
  if (!keep) await closeMic();
}
capture.on((msg) => {
  if (msg.type === "stop" || msg.type === "configure") generation++;
  const token = generation;
  serial = serial
    .then(async () => {
      if (msg.type === "configure") {
        await end(false);
        keep = msg.keep;
        selected = msg.device;
        await closeMic();
        if (keep) await openMic();
        await devices();
      }
      if (msg.type === "start") {
        recordingId = msg.id;
        await openMic();
        if (token !== generation) {
          if (!keep) await closeMic();
          return;
        }
        chunks = keep ? pre.map((a) => a.slice()) : [];
        if (keep && ctx) {
          const excess =
            chunks.reduce((n, a) => n + a.length, 0) -
            Math.round(ctx.sampleRate * 0.3);
          if (excess > 0 && chunks.length) chunks[0] = chunks[0].slice(excess);
        }
        total = chunks.reduce((n, a) => n + a.length, 0);
        recording = true;
      }
      if (msg.type === "stop") await end(msg.accept);
      if (msg.type === "devices") await devices();
    })
    .catch((e) => capture.send("error", String(e)));
});
navigator.mediaDevices.addEventListener("devicechange", () => void devices());
void devices();
