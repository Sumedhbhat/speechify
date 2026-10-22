import { readFileSync, writeFileSync } from 'node:fs';
const [source, task, count, output] = process.argv.slice(2);
const bytes = readFileSync(source);
const audio = new Float32Array(bytes.buffer.slice(bytes.byteOffset, bytes.byteOffset + bytes.byteLength));
function run(): boolean | Uint8Array {
  if (task === 'silence') {
    if (!audio.length) return true;
    let sum = 0;
    for (let i = 0; i < audio.length; i++) sum += Math.fround(audio[i] * audio[i]);
    return Math.fround(Math.sqrt(Math.fround(sum / audio.length))) < 0.004;
  }
  const pcm = new Int16Array(audio.length);
  for (let i = 0; i < audio.length; i++) {
    pcm[i] = Math.fround(Math.max(-1, Math.min(1, audio[i])) * 32767);
  }
  return new Uint8Array(pcm.buffer);
}
for (let i = 0; i < 30; i++) run();
let result: boolean | Uint8Array = false;
const start = process.hrtime.bigint();
for (let i = 0; i < Number(count); i++) result = run();
const elapsed = Number(process.hrtime.bigint() - start);
writeFileSync(output, typeof result === 'boolean' ? new Uint8Array([Number(result)]) : result);
console.log(JSON.stringify({ ns_per_op: elapsed / Number(count) }));
