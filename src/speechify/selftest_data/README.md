Both clips are LibriSpeech utterances (Panayotov et al., 2015, CC BY 4.0),
16 kHz mono. `speechify selftest` plays them in place of a microphone.

`librispeech-sample.wav` is what the self-test dictates, and the speech the
model worker warms up on. What's said:

> He hoped there would be stew for dinner, turnips and carrots and bruised
> potatoes and fat mutton pieces to be ladled out in thick, peppered,
> flour-fattened sauce.

`librispeech-quilter.wav` (dev-clean 1272-128104-0001) checks capital letters
and punctuation: Whisper large-v3-turbo used to return it all lowercase,
without the full stop. What's said:

> Nor is Mr. Quilter's manner less interesting than his matter.

LibriSpeech's own transcripts are uppercase without punctuation; the
punctuation above was added for Speechify's formatting check.
