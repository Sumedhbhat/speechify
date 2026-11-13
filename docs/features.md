# Features

This page describes what Speechify does to your words between the microphone and the text field. The examples are taken from the unit tests in `core/src/format/*.rs` and `core/src/pipeline.rs`, so they show real behaviour. Inputs are what the speech model heard, written as it writes them.

## The two cleanup layers

1. **The rule-based formatter** always runs. It is deterministic, fast and needs nothing installed.
2. **An optional language model** (Ollama by default, or any OpenAI-compatible server) rewrites the text first when it is enabled, reachable, and the dictation has at least 6 words (`rewrite.min_words`).

The model's answer is checked before it is used. It is thrown away, and the rule-based result is pasted, when:

- the request fails or takes longer than the timeout (6 seconds by default);
- the answer starts like a chat reply ("Sure", "Here is", "I'm sorry") and you did not;
- the answer is much longer than what you said, or fewer than half of its words were spoken;
- the model returns `EMPTY` even though the source contains meaningful content.

These checks catch a model that answers your dictation or drops its content instead of cleaning it. For example, the test feeds in "what is the capital of france please tell me", the model answers with a paragraph about Paris, and the pasted text is "What is the capital of france please tell me."

Whatever the model returns still goes through the final rule passes (number grouping, spelling, your dictionary and snippets, punctuation, tone). Code and terminal targets apply only your dictionary and snippets, preserving literal text.

The cleanup level in Settings has three values: None (paste what was recognised, apart from your dictionary and snippets), Light (the default) and Medium (the model may also tighten wording).

## What the formatter does

The passes run in this order: collapse spaces, remove fillers, apply self-corrections, spoken punctuation and breaks, numbers, lists, then spelling, dictionary and snippets, punctuation repair, and tone.

### Fillers and stutters

| You said | Typed |
| --- | --- |
| Um, I think, uh, we should go. | I think, we should go. |
| So uhm that works. Hmm, maybe. | So that works. Maybe. |
| I I think the the plan is is fine | I think the plan is fine |

Only hesitation sounds and a short list of words people repeat while thinking (the, a, I, to, and, we, is, it, and similar) are removed. "He had had enough, very very much." is left alone, because those repeats can be deliberate. Words like "umbrella" and "summer" are never touched.

### Self-corrections

| You said | Typed |
| --- | --- |
| Let's do coffee at 2 actually 3 | Let's do coffee at 3 |
| Let's do coffee at 2 pm, actually 3. | Let's do coffee at 3 pm. |
| Meet on Monday. No wait, Tuesday. | Meet on Tuesday. |
| Ship it in March, sorry, April. Thanks | Ship it in April. Thanks |
| I need five, no actually six copies | I need six copies |
| I'll go to the store, no wait, the mall. | I'll go to the mall. |
| We think, I mean, we know it works. | We know it works. |
| Send it today. Scratch that. Send it tomorrow. | Send it tomorrow. |

"Scratch that", "strike that", "delete that" and "never mind" erase the sentence they follow. Phrases like "no wait" and "no actually" are strong markers. Everyday words such as "actually", "sorry" and "I mean" only count as corrections when what follows clearly restates what came before, so "I actually enjoyed the film." and "Sorry for the delay." pass through unchanged.

### Spoken punctuation and line breaks

| You said | Typed |
| --- | --- |
| hello comma how are you question mark | Hello, how are you? |
| Great exclamation point Done full stop | Great! Done. |
| Note colon bring ID semicolon arrive early | Note: bring ID; arrive early |
| Hi Asha, new line. Thanks for the notes | Hi Asha<br>Thanks for the notes (two lines) |
| First part. New paragraph. Second part. | First part, a blank line, then Second part. |

"New line", "next line", "line break" and "skip a line" make one break. "New paragraph", "next paragraph" and "start a new paragraph" make a blank line. When you talk about punctuation instead, the words stay: "Remove the comma after hello" and "Add a question mark there" are typed as spoken.

### Numbers

Spoken numbers become digits. By default they are grouped the Indian way, with lakh and crore. Switch to international grouping in Settings or on the Style page.

| You said | Typed |
| --- | --- |
| twenty five people | 25 people |
| one hundred and fifty items | 150 items |
| two lakh fifty thousand | 2,50,000 |
| three crore twenty lakh | 3,20,00,000 |
| five lakh | 5 lakh |
| twelve crores | 12 crore |
| two point five lakh | 2.5 lakh |
| in nineteen ninety nine | in 1999 |
| by twenty twenty six. | by 2026. |
| version three point one four | version 3.14 |
| call nine eight four five zero one | call 984501 |
| five percent | 5% |
| five hundred rupees | ₹500 |
| 3 lakh rupees | ₹3 lakh |
| twenty dollars. | $20. |

Small and ordinary uses are left as words: "one of the two options", "the twenty first century" and "bread and butter" do not change. Digits already in the text are regrouped to match your setting: "It cost 250,000 and 1,200." becomes "It cost 2,50,000 and 1200." in Indian grouping, and "It cost 2,50,000." becomes "It cost 250,000." in international grouping.

### Lists

A spoken enumeration becomes a list. Ordinary sentences that happen to contain number words are left alone.

| You said | Typed |
| --- | --- |
| um my plan for today is one, buy milk. two, call mom. three, pay the bills | My plan for today is:<br>1. Buy milk<br>2. Call mom<br>3. Pay the bills |
| We need three things. First, a laptop. Second, a charger. And third, a bag. Let me know if that works. | We need three things:<br>1. A laptop<br>2. A charger<br>3. A bag<br>(blank line)<br>Let me know if that works. |
| Agenda: bullet point budget, next bullet hiring, bullet roadmap. | Agenda:<br>- Budget<br>- Hiring<br>- Roadmap |

Each list item goes on its own line. Not lists: "I have one dog and two cats", "The first time I saw two of them", "One, two." and "a bullet train".

### Spelling

With the default Indian setting, American spellings are converted to Indian and British ones, and case is kept.

| You said | Typed |
| --- | --- |
| My favorite color is gray. | My favourite colour is grey. |
| Please organize and prioritize the organization's work. | Please organise and prioritise the organisation's work. |
| We analyzed it while traveling to the center. | We analysed it while travelling to the centre. |

Words that only look similar are left alone ("prize", "program", "colorado"). Choose American spelling in Settings to turn this off.

### Several passes together

"the color budget is two lakh fifty thousand rupees, actually three lakh rupees" becomes "The colour budget is ₹3 lakh." The correction replaces the whole earlier amount, the amount is formatted, and the spelling is converted.

## Tone per destination

Speechify notes the focused app and window title when you start dictating, and sorts it into a category. Each category has a style.

| Category | Examples | Default style |
| --- | --- | --- |
| Personal messages | WhatsApp, Telegram, Signal, Messages, Discord | Casual |
| Work messages | Slack, Teams, Zoom, Linear, Jira | Formal |
| Email | Outlook, Thunderbird, Gmail in a browser tab | Formal |
| AI prompts | ChatGPT, Claude, Gemini, Copilot | Formal (the "everything else" style) |
| Documents | Word, Notion, Obsidian, Notepad | Formal |
| Code | VS Code, Cursor, JetBrains IDEs, Xcode | none, kept as spoken |
| Terminal | Terminal, PowerShell, iTerm, Konsole | none, kept as spoken |
| Other | anything unrecognised | Formal |

App names and process names are matched first, then the window title, which is how web apps in a browser are told apart. On the Style page you can add your own rules ("app name contains" ... "treat as" a category). Your rules win over the built-in ones.

The four styles:

| Style | Input | Output |
| --- | --- | --- |
| Formal | See you at the office | See you at the office. |
| Casual | On my way. See you soon. | On my way. See you soon |
| Very casual | Hey. I'm late. Tell Asha I said hi. | hey. I'm late. tell Asha I said hi |
| Excited | That works. | That works! |

Casual drops the last full stop of a short message. Very casual lowercases the start of sentences but keeps names, acronyms and "I" ("NASA called. GitHub is down." keeps NASA and GitHub). Excited ends on an exclamation mark. Questions, ellipses and list lines are not changed.

Code and Terminal are "literal": the rule-based formatter preserves case, numbers, spacing and identifiers, and the paste has no trailing space. Only dictionary and snippet replacements apply. "Push the color fix to get hub." with a dictionary entry for "get hub" becomes "Push the color fix to GitHub." `git status` and `printf '1,234'` keep their original case and numbers.

In a Linux terminal the paste uses Ctrl+Shift+V. In a code editor on Windows it uses Shift+Insert. Elsewhere it uses Ctrl+V, or Cmd+V on a Mac. Remote-desktop apps get a longer wait before your clipboard is restored.

A trailing space is added after pasted text so the next dictation continues the line. It is left off in code and terminals, and when the text has line breaks.

## Dictionary

The Dictionary page holds two kinds of entry:

- **A word or name** you want spelled a certain way, such as "GitHub". It is whole-word and case-insensitive, so it also fixes capitalisation.
- **A fix** from what the recogniser hears to what you meant, such as "sumit" to "Sumedh".

Matching rules, from the tests: whole words only, any case, longest match first. With the rules sumit to Sumedh, "sumit bat" to Sumedh Bhat and k8s to Kubernetes, "Ask Sumit Bat about K8S, not sumit." becomes "Ask Sumedh Bhat about Kubernetes, not Sumedh." Contractions and partial words are skipped: with a rule for "can", "I can't scan the can." becomes "I can't scan the tin."

The terms also go to recognisers that support a glossary and to the language model as a vocabulary list. Recognition hints depend on the selected model; they do not guarantee a correction. Entries are counted each time they fire. Entries can be added by you, learned from your edits, or learned by the teacher. If you delete a learned entry, it is remembered as forgotten and is not suggested again.

## Snippets

A snippet types a longer text when you say a trigger phrase. With "my email address" set to `me@example.com`, "Send it to my email, address please." becomes "Send it to me@example.com please." Triggers match through the punctuation a recogniser adds. Snippets work in every cleanup level, including None, and a duplicate trigger is refused when you save.

## Press enter

Turn on "Say press enter to send" in Settings. If a dictation ends with "press enter", the words are removed and Enter is pressed after the paste. "search for flights, press enter." types "Search for flights." and presses Enter. Saying only "press enter" presses Enter and types nothing. In the middle of a sentence ("press enter to continue") the words are left alone. It is off by default.

## Command Mode

Select text in any app, hold the command shortcut, say an instruction, and let go. Speechify copies the selection, sends the text and your instruction to the language model, and pastes the answer over the selection.

- With the test case: selection "helo wrld", instruction "fix the typos", model answer `"Hello world"`, pasted text `Hello world`. Quotes, code fences and "RESULT:" labels around a model answer are stripped.
- With nothing selected, the model writes what the instruction asks for.
- It needs the language model. If it is off or unreachable you get an error and the recording is kept for Undo.
- If the model returns nothing, you get "The language model returned nothing for that command".
- Reading the selection does not work on Wayland.

## Recognition guards

- A recording with no speech in it gives "No speech heard. Is your microphone muted?" Silence is trimmed before recognition, so the model is not given noise to invent words from.
- Sound tags such as `[BLANK_AUDIO]` and `(music)` are stripped.
- Phrases that speech models make up from silence ("Thank you.", "Thanks for watching", "you", "bye") are dropped when there was too little speech to have said them. A real "thank you" of normal length is kept.
- Whisper always receives a short punctuated prompt in Indian English, plus your glossary. Without a prompt Whisper sometimes drops capitals and punctuation for a whole dictation.

## History

Each dictation is stored with the heard text, the final text, the app, timings, and the recording if you keep audio. On the Home page you can search, play the recording, copy, flag for review, edit and delete. Details show the app, category, models, timings and fixes made.

Editing a dictation teaches Speechify at once: small word swaps become dictionary entries, and the edit becomes a worked example for the language model's prompt. A rewrite of most of the sentence teaches nothing about spelling, so it is not turned into dictionary entries. Common words swapped for other common words count as editing, not correcting.

Retention in Settings is Keep (default), Day (delete dictations older than a day) or Never (store nothing, which also turns off history, learning and the nightly review).

## The nightly teacher

After the set hour (2 am by default) and once a day, when the app is idle, the teacher reviews dictations that have a recording and have not been reviewed. It loads the larger speech model (Whisper large-v3) and, if Ollama has it, the larger language model (`qwen3.5:9b-q4_K_M`), then for up to 200 dictations:

1. Transcribes the recording again with the larger speech model.
2. Scores the everyday model's transcript against it as a word error rate.
3. Runs the cleanup on the teacher's transcript, always with the language model, to get an ideal result.
4. Compares what the everyday model heard with what the teacher heard. Reviews with more than 40% word disagreement in either direction do not teach corrections, vocabulary or automatic worked examples. Normally a correction must be seen twice (`learn_after`). A single sighting can qualify when the student heard a capitalised term, its spelling is close to the correction, and the corrected spelling was already independently known. Title case alone is insufficient.

5. If an eligible ideal result (or your own edit, which outranks it) differs from what was typed, it is saved as a worked example. Up to three examples, preferring the same category of app, go into the everyday model's prompt. The newest 200 are kept.

Notable terms that both recognisers heard are added as plain vocabulary when auto-learning is enabled. This also supplies spelling evidence for future mishearings. The formatter applies exact dictionary matches; it does not fuzzy-match ordinary words. Deleted entries remain forgotten.

Automatic worked examples must have plausible length and word overlap with both transcripts. Model-generated lists require evidence of a spoken enumeration. User hand-edits remain trusted. Silent history recordings are skipped before teacher recognition.

A review stops as soon as you start dictating. Days with nothing to review do not load the large models. Press "Review now" on the Learning page to run it by hand.

Nothing is fine-tuned on your machine. "Export training data" writes `training.jsonl` to the app data folder, one line per reviewed dictation with the audio file name, what each model heard, what was typed, the ideal text, and whether you corrected it. Each row includes a `command` flag; command rows retain recognition data and have `ideal: null` because their generated output is not a cleanup target. You can use that to fine-tune models elsewhere. It needs the recordings, so keep audio on if you want it.

Turn off "Learn from my corrections" in Settings to stop dictionary entries being added automatically.

## Insights

The Insights page is computed from the dictations table:

- words and dictations, minutes spoken, speed in words per minute, and minutes saved against typing at 40 words per minute;
- words today and this week, current and longest streak, active days;
- an activity calendar of 26 weeks, shaded by words per day (1 to 249, 250 to 499, 500 to 749, 750 or more);
- words by hour of day and by weekday;
- shares by app and by category;
- latency from releasing the key to the text appearing (typical and slowest 5%), and the share of dictations the language model rewrote;
- teacher agreement: one minus the average word error rate against the teacher, over reviewed dictations. The legacy accuracy field is null because this is not verified accuracy.

## Other behaviour

- **Hands-free limit.** A warning at 19 minutes, and recording stops at 20.
- **Still working.** Starting a new dictation while the last is processing shows a notice and does nothing.
- **Failures keep the recording.** If recognition or Command Mode fails, the audio is kept and the bar offers Undo to try again.
- **Sounds.** A chime on start, stop and error, on by default. Mute-while-dictating is off by default.
- **Launch at login** is on by default and starts the app in the tray. Closing the main window keeps dictation running.
- **Notes** is a place to dictate into inside the app. It saves as you type.
- **Remote speech server.** If you set an address on the Models page, an OpenAI-compatible `/audio/transcriptions` server is used instead of the local model.
- **Languages.** Settings offers automatic detection, English, Hindi, Bengali, Tamil, Telugu, Marathi, Gujarati, Kannada, Malayalam, Spanish, French, German, Portuguese, Japanese and Chinese. The formatter's rules (fillers, numbers, lists, corrections) are written for English.

## Evaluation-driven safeguards (11 November 2026)

Quiet recordings are detected relative to their own noise floor and peak frame energy. Already-detected quiet speech gets bounded gain before recognition. Silence, steady low-level room noise and isolated clicks are rejected; this energy heuristic is not a general-purpose voice activity model.

Self-corrections recognise May with calendar context or mid-sentence capitals. Ordinary uses of “may” stay intact. Layout noun phrases such as “a new line of products” and “the next paragraph” remain text, while “milk new line eggs” inserts a break.

After two consecutive failed or rejected cleanup rewrites, the controller uses rules immediately for one minute, then probes the model again. A failed probe renews the cooldown; success or a rewrite-settings change resets it. Command Mode still explicitly invokes the model. `Controller::rewrite_retry_at_ms` exposes cooldown status; the interface has not yet been wired to display it.

The core now returns `teacher_agreement` separately and keeps the legacy `accuracy` field null. Agreement with a larger model is not measured recognition accuracy. The existing interface still has its old accuracy heading and empty-state explanation; those labels need a separate interface change.
