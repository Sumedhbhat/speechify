"""No-inference control; not used to claim ASR/application performance."""
import os
class Echo:
    device = 'no-inference-control'
    def __init__(self, repo, opts):
        self.text = os.environ['BENCH_ECHO_TEXT']
    def transcribe(self, audio, prompt):
        return self.text
