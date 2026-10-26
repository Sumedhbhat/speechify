import { DatabaseSync, backup } from "node:sqlite";
import fs from "node:fs";
import path from "node:path";
import { randomUUID } from "node:crypto";
import { atomic, owned, edits, label } from "./core";
export class Store {
  db: DatabaseSync;
  audio: string;
  constructor(public root: string) {
    fs.mkdirSync(root, { recursive: true });
    this.audio = path.join(root, "audio");
    fs.mkdirSync(this.audio, { recursive: true });
    this.db = new DatabaseSync(path.join(root, "learning.db"));
    this.db.exec(
      `PRAGMA journal_mode=WAL; CREATE TABLE IF NOT EXISTS samples(id INTEGER PRIMARY KEY,ts REAL NOT NULL,audio TEXT,raw TEXT NOT NULL,pasted TEXT NOT NULL,corrected TEXT,excluded INTEGER NOT NULL DEFAULT 0,teacher_text TEXT,teacher_model TEXT,teacher_confidence REAL,teacher_ts REAL,teacher_note TEXT);CREATE TABLE IF NOT EXISTS rules(wrong TEXT NOT NULL,right TEXT NOT NULL,count INTEGER NOT NULL,PRIMARY KEY(wrong,right));CREATE TABLE IF NOT EXISTS vocab(term TEXT PRIMARY KEY,count INTEGER NOT NULL);CREATE TABLE IF NOT EXISTS learn_state(key TEXT PRIMARY KEY,value TEXT NOT NULL);`,
    );
  }
  rows(search = "", filter = "all") {
    return this.db
      .prepare(
        "SELECT * FROM samples WHERE coalesce(corrected,pasted) LIKE ? ORDER BY ts DESC LIMIT 500",
      )
      .all("%" + search + "%")
      .filter(
        (r: any) =>
          filter === "all" ||
          (filter === "edited" && r.corrected !== null) ||
          (filter === "unedited" && r.corrected === null) ||
          (filter === "excluded" && r.excluded),
      );
  }
  add(wav: Buffer, raw: string, pasted: string, keep = true) {
    const name = keep ? randomUUID() + ".wav" : null;
    if (name) atomic(owned(this.audio, name), wav);
    return Number(
      this.db
        .prepare("INSERT INTO samples(ts,audio,raw,pasted) VALUES(?,?,?,?)")
        .run(Date.now() / 1000, name, raw, pasted).lastInsertRowid,
    );
  }
  get(id: number) {
    const r = this.db.prepare("SELECT * FROM samples WHERE id=?").get(id);
    if (!r) throw Error("Recording not found");
    return r as any;
  }
  correct(id: number, text: string) {
    text = text.trim();
    if (!text) throw Error("Correction cannot be empty");
    const row = this.get(id);
    const before = row.corrected ?? row.pasted;
    this.db.exec("BEGIN");
    try {
      this.db
        .prepare("UPDATE samples SET corrected=? WHERE id=?")
        .run(text === row.pasted ? null : text, id);
      if (text !== before)
        for (const [wrong, right] of edits(before, text)) {
          this.db
            .prepare(
              "INSERT INTO rules VALUES(?,?,1) ON CONFLICT(wrong,right) DO UPDATE SET count=count+1",
            )
            .run(wrong, right);
          this.db
            .prepare(
              "INSERT INTO vocab VALUES(?,1) ON CONFLICT(term) DO UPDATE SET count=count+1",
            )
            .run(right);
        }
      this.db.exec("COMMIT");
    } catch (e) {
      this.db.exec("ROLLBACK");
      throw e;
    }
  }
  rules() {
    return this.db.prepare("SELECT * FROM rules ORDER BY count DESC").all();
  }
  vocab() {
    return this.db.prepare("SELECT * FROM vocab ORDER BY count DESC").all();
  }
  apply(text: string, threshold: number) {
    for (const r of this.rules() as any[]) {
      if (r.count < threshold) continue;
      const esc = r.wrong.replace(/[.*+?^${}()|[\]\\]/g, "\\$&");
      text = text.replace(
        new RegExp("(?<![\\p{L}\\p{N}])" + esc + "(?![\\p{L}\\p{N}])", "giu"),
        () => r.right,
      );
    }
    return text;
  }
  forget(term: string) {
    this.db
      .prepare("DELETE FROM rules WHERE wrong=? OR lower(right)=?")
      .run(term.toLowerCase(), term.toLowerCase());
    this.db
      .prepare("DELETE FROM vocab WHERE lower(term)=?")
      .run(term.toLowerCase());
  }
  exclude(id: number, value: boolean) {
    this.db.prepare("UPDATE samples SET excluded=? WHERE id=?").run(+value, id);
  }
  remove(id: number) {
    const row = this.get(id);
    if (row.audio) fs.rmSync(owned(this.audio, row.audio), { force: true });
    this.db.prepare("DELETE FROM samples WHERE id=?").run(id);
  }
  audioFile(id: number) {
    const row = this.get(id);
    if (!row.audio) throw Error("Audio was not retained");
    return owned(this.audio, row.audio);
  }
  state(key: string, fallback: any = null) {
    const r = this.db
      .prepare("SELECT value FROM learn_state WHERE key=?")
      .get(key);
    return r ? JSON.parse(String(r.value)) : fallback;
  }
  setState(key: string, value: any) {
    this.db
      .prepare(
        "INSERT INTO learn_state VALUES(?,?) ON CONFLICT(key) DO UPDATE SET value=excluded.value",
      )
      .run(key, JSON.stringify(value));
  }
  teacher(
    id: number,
    text: string,
    model: string,
    confidence: number,
    note: string | null,
  ) {
    this.db
      .prepare(
        "UPDATE samples SET teacher_text=?,teacher_model=?,teacher_confidence=?,teacher_ts=?,teacher_note=? WHERE id=?",
      )
      .run(text, model, confidence, Date.now() / 1000, note, id);
  }
  export(dest: string, min = 0.7) {
    fs.mkdirSync(dest, { recursive: true });
    let csv = "file_name,transcription\n",
      n = 0;
    for (const r of this.db
      .prepare("SELECT * FROM samples ORDER BY ts")
      .all() as any[]) {
      const text = label(r, min);
      if (!text) continue;
      const name = randomUUID() + ".wav";
      fs.copyFileSync(owned(this.audio, r.audio), path.join(dest, name));
      csv += name + ',"' + text.replace(/"/g, '""') + '"\n';
      n++;
    }
    atomic(path.join(dest, "metadata.csv"), csv);
    return n;
  }
  async import(source: string) {
    if (path.resolve(source) === path.resolve(this.root))
      throw Error("Source is already candidate data");
    const src = new DatabaseSync(path.join(source, "learning.db"), {
      readOnly: true,
    });
    const staging = path.join(this.root, "import-" + randomUUID() + ".db");
    try {
      await backup(src, staging);
      const copy = new DatabaseSync(staging);
      const copied: string[] = [];
      this.db.exec("BEGIN");
      try {
        const cols = new Set(
          copy
            .prepare("PRAGMA table_info(samples)")
            .all()
            .map((r) => r.name),
        );
        for (const [name, type] of Object.entries({
          excluded: "INTEGER DEFAULT 0",
          teacher_text: "TEXT",
          teacher_model: "TEXT",
          teacher_confidence: "REAL",
          teacher_ts: "REAL",
          teacher_note: "TEXT",
        }))
          if (!cols.has(name))
            copy.exec(`ALTER TABLE samples ADD COLUMN ${name} ${type}`);
        for (const row of copy
          .prepare("SELECT * FROM samples")
          .all() as any[]) {
          let audio = null;
          if (row.audio) {
            const sourceAudio = owned(path.join(source, "audio"), row.audio);
            audio = randomUUID() + ".wav";
            const target = owned(this.audio, audio);
            fs.copyFileSync(sourceAudio, target);
            copied.push(target);
          }
          this.db
            .prepare(
              "INSERT INTO samples(ts,audio,raw,pasted,corrected,excluded,teacher_text,teacher_model,teacher_confidence,teacher_ts,teacher_note) VALUES(?,?,?,?,?,?,?,?,?,?,?)",
            )
            .run(
              row.ts,
              audio,
              row.raw,
              row.pasted,
              row.corrected,
              row.excluded,
              row.teacher_text,
              row.teacher_model,
              row.teacher_confidence,
              row.teacher_ts,
              row.teacher_note,
            );
        }
        for (const r of copy.prepare("SELECT * FROM rules").all())
          this.db
            .prepare("INSERT OR IGNORE INTO rules VALUES(?,?,?)")
            .run(r.wrong, r.right, r.count);
        for (const r of copy.prepare("SELECT * FROM vocab").all())
          this.db
            .prepare("INSERT OR IGNORE INTO vocab VALUES(?,?)")
            .run(r.term, r.count);
        // Imported version paths are preserved as metadata, never adopted or pruned.
        this.setState(
          "imported_learn_state",
          copy.prepare("SELECT * FROM learn_state").all(),
        );
        this.db.exec("COMMIT");
      } catch (e) {
        this.db.exec("ROLLBACK");
        for (const file of copied) fs.rmSync(file, { force: true });
        throw e;
      } finally {
        copy.close();
      }
    } finally {
      src.close();
      fs.rmSync(staging, { force: true });
    }
  }
  close() {
    this.db.close();
  }
}
