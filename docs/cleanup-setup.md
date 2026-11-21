# Optional language-model cleanup

Speechify can dictate as soon as its speech model is installed. Fresh installs use built-in formatting until you turn on **Models → Tidy text with a language model**. The language model also enables Command Mode. It runs separately from the speech recognizer.

## Set up llama-server

1. Download the llama.cpp archive for your machine from the [official releases](https://github.com/ggml-org/llama.cpp/releases). Extract the entire archive and keep its libraries alongside the executable.
   - **macOS:** choose **Apple Silicon (arm64)** for an M-series Mac or **Intel (x64)** for an Intel Mac. Check the server archive’s macOS requirement. The tested b12521 Apple Silicon build requires macOS 13.3 or newer. On an older Mac, you can use built-in cleanup or connect to a compatible server running on another machine. A remote server receives the dictated text and cleanup context.
   - **Windows:** choose **Windows x64 (CPU)** for the simplest setup on an Intel or AMD PC. Open PowerShell in the extracted folder.
   - **Linux:** choose the **Ubuntu CPU** archive for your architecture on Ubuntu, or use a build for your distribution. Open a terminal in the extracted folder.
2. Download [Llama-3.2-3B-Instruct-Q4_K_M.gguf](https://huggingface.co/bartowski/Llama-3.2-3B-Instruct-GGUF/resolve/5ab33fa94d1d04e903623ae72c95d1696f09f9e8/Llama-3.2-3B-Instruct-Q4_K_M.gguf?download=true), about 2 GB, and put it in the folder with the server executable.
3. Run the command for your platform and keep the terminal open when dictating.

   **macOS and Linux**

   ```sh
   ./llama-server -m Llama-3.2-3B-Instruct-Q4_K_M.gguf --alias llama3.2:3b -c 4096 --host 127.0.0.1 --port 18080 --log-disable
   ```

   **Windows PowerShell**

   ```powershell
   .\llama-server.exe -m Llama-3.2-3B-Instruct-Q4_K_M.gguf --alias llama3.2:3b -c 4096 --host 127.0.0.1 --port 18080 --log-disable
   ```

4. Open **Models → Language model** in Speechify. Confirm the server is `http://127.0.0.1:18080/v1` and the model is `llama3.2:3b`. Click **Refresh**. Once it says **Connected**, turn on **Tidy text with a language model** and try a sentence of at least six words.

If the server won’t start, first check that the model file is beside the executable and that you opened the terminal in that folder. If port 18080 is already in use, choose a free port in both the command and the Speechify server field. Restart the command when you next want to use model cleanup. If the server is unavailable or a response is rejected, Speechify uses the built-in formatter for ordinary dictation.

## Use an existing server

- **Ollama:** Start Ollama, set the server to `http://127.0.0.1:11434`, then enter an installed model name or use **Download with Ollama**. Click **Refresh**, then enable model cleanup.
- **Other OpenAI-compatible server:** Load the model in that server. Enter its base address ending in `/v1` and the model ID it exposes. Click **Refresh**, then enable model cleanup.
- **Updating Speechify:** Your chosen server and model carry over. To switch to llama-server, follow the setup above and enter both values in Models.
