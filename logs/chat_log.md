[2026-03-29 16:42:22.587] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-03-29 16:42:22.588] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 16:42:22.589] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler spawned — model='qwen3.5:4b' url='http://192.168.1.149:11434'"}
[2026-03-29 16:42:22.589] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-03-29 16:42:22.601] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 16:42:22.619] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 16:42:22.620] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama reachable at http://192.168.1.149:11434 ✅"}
[2026-03-29 16:42:22.622] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama available models: [qwen3.5:4b, llama3:latest]"}
[2026-03-29 16:42:22.622] logger -> web_interface: {"type":"log","level":"info","msg":"Configured model 'qwen3.5:4b' is available ✅"}
[2026-03-29 16:44:54.735] web_user -> ollama: hi
[2026-03-29 16:44:54.742] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_user' data='hi'"}
[2026-03-29 16:44:54.744] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 16:44:54.747] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-03-29 16:46:20.200] ollama -> cpu: Hi there! 😊 How can I assist you today? Feel free to ask anything or share what's on your mind!
[2026-03-29 16:46:20.208] ollama_lan -> cpu: {"type":"llm_response","correlation_id":0,"msg":"Hi there! 😊 How can I assist you today? Feel free to ask anything or share what's on your mind!"}
[2026-03-29 16:46:20.210] ollama -> web_interface: {"type":"ollama_response","reply_to":"web_user","msg":"Hi there! 😊 How can I assist you today? Feel free to ask anything or share what's on your mind!"}
[2026-03-29 16:46:20.211] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 1/3 — 98 chars"}
[2026-03-29 16:46:20.212] logger -> web_interface: {"type":"log","level":"error","msg":"Failed to parse LLM request payload: expected value at line 1 column 1"}
[2026-03-29 17:26:47.209] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-03-29 17:26:47.228] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 17:26:47.232] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler spawned — model='qwen3.5:4b' url='http://192.168.1.149:11434'"}
[2026-03-29 17:26:47.234] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-03-29 17:26:47.255] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 17:26:47.258] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 17:26:47.261] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama reachable at http://192.168.1.149:11434 ✅"}
[2026-03-29 17:26:47.264] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama available models: [qwen3.5:4b, llama3:latest]"}
[2026-03-29 17:26:47.267] logger -> web_interface: {"type":"log","level":"info","msg":"Configured model 'qwen3.5:4b' is available ✅"}
[2026-03-29 19:01:51.159] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-03-29 19:01:51.163] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 19:01:51.166] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler spawned — model='qwen3.5:4b' url='http://192.168.1.149:11434'"}
[2026-03-29 19:01:51.169] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-03-29 19:01:51.172] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-03-29 19:01:51.174] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 19:01:51.177] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama reachable at http://192.168.1.149:11434 ✅"}
[2026-03-29 19:01:51.180] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama available models: [qwen3.5:4b, llama3:latest]"}
[2026-03-29 19:01:51.183] logger -> web_interface: {"type":"log","level":"info","msg":"Configured model 'qwen3.5:4b' is available ✅"}
[2026-03-29 19:09:20.527] web_interface -> cpu: {"type":"user_input","content_type":"text","content":"good eveinghi","correlation_id":1774825760527,"metadata":{}}
[2026-03-29 19:09:20.536] cpu -> ollama: {"type":"llm_request","prompt":"good eveinghi","correlation_id":1774825760527}
[2026-03-29 19:09:20.538] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='cpu' data='{\"type\":\"llm_request\",\"prompt\":\"good eveinghi\",\"correlation_id\":1774825760527}'"}
[2026-03-29 19:09:20.540] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 19:09:20.543] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-03-29 19:09:30.324] web_interface -> cpu: {"type":"user_input","content_type":"text","content":"hi","correlation_id":1774825770324,"metadata":{}}
[2026-03-29 19:09:30.332] cpu -> ollama: {"type":"llm_request","prompt":"hi","correlation_id":1774825770324}
[2026-03-29 19:09:30.334] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='cpu' data='{\"type\":\"llm_request\",\"prompt\":\"hi\",\"correlation_id\":1774825770324}'"}
[2026-03-29 19:09:30.343] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-03-29 19:09:30.364] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-03-29 19:10:47.210] ollama -> cpu: Good evening! How can I help you today?
[2026-03-29 19:10:47.217] ollama_lan -> cpu: {"type":"llm_response","correlation_id":1774825760527,"msg":"Good evening! How can I help you today?"}
[2026-03-29 19:10:47.218] ollama -> web_interface: {"type":"ollama_response","reply_to":"cpu","msg":"Good evening! How can I help you today?"}
[2026-03-29 19:10:47.219] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 1/3 — 39 chars"}
[2026-03-29 19:11:00.380] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat))"}
[2026-03-29 19:11:00.388] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-03-29 19:12:30.039] ollama -> cpu: Hello! How can I help you today?
[2026-03-29 19:12:30.049] ollama_lan -> cpu: {"type":"llm_response","correlation_id":1774825770324,"msg":"Hello! How can I help you today?"}
[2026-03-29 19:12:30.051] ollama -> web_interface: {"type":"ollama_response","reply_to":"cpu","msg":"Hello! How can I help you today?"}
[2026-03-29 19:12:30.052] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 2/3 — 32 chars"}
