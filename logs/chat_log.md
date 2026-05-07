
[2026-04-11 21:53:57.487] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-04-11 21:53:57.494] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-11 21:53:57.495] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler spawned — model='qwen3.5:4b' url='http://192.168.1.149:11434'"}
[2026-04-11 21:53:57.496] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-04-11 21:53:57.497] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-11 21:53:57.498] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-11 21:53:57.499] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama reachable at http://192.168.1.149:11434 ✅"}
[2026-04-11 21:53:57.500] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama available models: [qwen3.5:4b, llama3:latest]"}
[2026-04-11 21:53:57.501] logger -> web_interface: {"type":"log","level":"info","msg":"Configured model 'qwen3.5:4b' is available ✅"}
[2026-04-11 21:54:35.008] web_interface -> cpu: {"type":"user_input","content_type":"text","content":"hi","correlation_id":1775958875008,"metadata":{}}
[2026-04-11 21:54:35.016] cpu -> ollama_lan: {"type":"llm_request","correlation_id":1775958875008,"prompt":"hi"}
[2026-04-12 13:41:26.402] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-04-12 13:41:26.404] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 13:41:26.405] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'server' spawned on bus channel 'ollama_server'"}
[2026-04-12 13:41:26.405] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'local3090' spawned on bus channel 'ollama_local3090'"}
[2026-04-12 13:41:26.407] logger -> web_interface: {"type":"log","level":"info","msg":"Gemini handler spawned on bus channel 'gemini'"}
[2026-04-12 13:41:26.408] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-04-12 13:41:26.409] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 13:41:26.410] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'server' reachable at http://192.168.1.149:11434 ✅"}
[2026-04-12 13:41:26.417] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 13:41:26.825] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 13:41:26.826] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'local3090' reachable at http://192.168.1.196:11434 ✅"}
[2026-04-12 13:41:49.627] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"hi","correlation_id":1776015709627}
[2026-04-12 13:41:49.634] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776015709627}'"}
[2026-04-12 13:41:49.645] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 13:41:49.657] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:7b' is available in Ollama ✅"}
[2026-04-12 13:41:49.663] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 13:41:49.663] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 13:41:49.665] logger -> web_interface: {"type":"log","level":"error","msg":"Aborting retries — model 'qwen3.5:7b' not found in Ollama. Check config.toml [ollama] model or run `ollama pull qwen3.5:7b`."}
[2026-04-12 13:41:49.666] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 13:42:05.627] web_interface -> ollama_server: {"type":"chat_request","prompt":"hi","correlation_id":1776015725627}
[2026-04-12 13:42:05.629] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776015725627}'"}
[2026-04-12 13:42:05.632] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 13:42:05.636] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-04-12 13:43:15.474] ollama_lan -> cpu: {"type":"llm_response","correlation_id":1776015725627,"msg":"Hi there! How can I help you today?"}
[2026-04-12 13:43:15.474] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 1/3 — 35 chars"}
[2026-04-12 13:43:15.482] cpu -> web_interface: {"type":"llm_output","correlation_id":1776015725627,"msg":"Hi there! How can I help you today?"}[2026-04-12 13:43:15.482] ollama_server -> web_interface: {"type":"ollama_response","llm":"server","reply_to":"web_interface","msg":"Hi there! How can I help you today?"}

[2026-04-12 13:46:02.614] config -> web_interface: {"type":"config_status","status":"success","msg":"Config saved successfully. Restart the bot to apply changes."}
[2026-04-12 13:47:14.231] web_interface -> ollama_server: {"type":"chat_request","prompt":"list out the tools you have","correlation_id":1776016034231}
[2026-04-12 13:47:14.244] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"list out the tools you have\",\"correlation_id\":1776016034231}'"}
[2026-04-12 13:47:14.247] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 13:47:14.260] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-04-12 13:48:44.272] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat))"}
[2026-04-12 13:48:44.272] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-04-12 13:50:16.804] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 2/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat))"}
[2026-04-12 13:50:16.804] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 2/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-04-12 13:51:49.328] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}[2026-04-12 13:51:49.328] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 3/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}

[2026-04-12 13:51:49.336] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-04-12 14:00:54.932] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"list tools you have","correlation_id":1776016854932}
[2026-04-12 14:00:54.944] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"list tools you have\",\"correlation_id\":1776016854932}'"}
[2026-04-12 14:00:54.955] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:00:54.966] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:7b' is available in Ollama ✅"}
[2026-04-12 14:00:54.972] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:00:54.972] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:00:54.974] logger -> web_interface: {"type":"log","level":"error","msg":"Aborting retries — model 'qwen3.5:7b' not found in Ollama. Check config.toml [ollama] model or run `ollama pull qwen3.5:7b`."}
[2026-04-12 14:00:54.975] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:01:08.979] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"hi","correlation_id":1776016868979}
[2026-04-12 14:01:08.986] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776016868979}'"}
[2026-04-12 14:01:08.995] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:01:09.008] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:7b' is available in Ollama ✅"}
[2026-04-12 14:01:09.015] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:01:09.015] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:01:09.016] logger -> web_interface: {"type":"log","level":"error","msg":"Aborting retries — model 'qwen3.5:7b' not found in Ollama. Check config.toml [ollama] model or run `ollama pull qwen3.5:7b`."}
[2026-04-12 14:01:09.017] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): Ollama returned HTTP 404 Not Found: {\"error\":\"model 'qwen3.5:7b' not found\"}"}
[2026-04-12 14:01:25.925] config -> web_interface: {"type":"config_status","status":"success","msg":"Config saved successfully. Restart the bot to apply changes."}
[2026-04-12 14:01:35.401] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-04-12 14:01:35.403] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 14:01:35.403] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'server' spawned on bus channel 'ollama_server'"}
[2026-04-12 14:01:35.404] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'local3090' spawned on bus channel 'ollama_local3090'"}
[2026-04-12 14:01:35.405] logger -> web_interface: {"type":"log","level":"info","msg":"Gemini handler spawned on bus channel 'gemini'"}
[2026-04-12 14:01:35.406] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-04-12 14:01:35.407] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 14:01:35.408] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 14:01:35.409] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'server' reachable at http://192.168.1.149:11434 ✅"}
[2026-04-12 14:01:35.413] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:01:35.414] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'local3090' reachable at http://192.168.1.196:11434 ✅"}
[2026-04-12 14:01:48.464] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"hi","correlation_id":1776016908464}
[2026-04-12 14:01:48.471] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776016908464}'"}
[2026-04-12 14:01:48.480] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:01:48.491] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:27b' is available in Ollama ✅"}
[2026-04-12 14:03:18.506] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat))"}
[2026-04-12 14:03:18.506] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:04:17.176] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"hi","correlation_id":1776017057176}
[2026-04-12 14:04:17.184] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776017057176}'"}
[2026-04-12 14:04:17.196] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:04:17.208] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:27b' is available in Ollama ✅"}
[2026-04-12 14:04:51.037] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 2/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat))"}
[2026-04-12 14:04:51.038] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 2/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:04:51.331] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (Ollama returned HTTP 500 Internal Server Error: {\"error\":\"unexpected server status: llm server loading model\"})"}
[2026-04-12 14:04:51.331] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: Ollama returned HTTP 500 Internal Server Error: {\"error\":\"unexpected server status: llm server loading model\"}"}
[2026-04-12 14:06:23.579] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 3/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:06:23.589] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:06:23.591] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:06:23.844] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 2/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat))"}
[2026-04-12 14:06:23.844] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 2/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:07:16.612] ollama_lan -> cpu: {"type":"llm_response","correlation_id":1776017057176,"msg":"Hello! How can I help you today?"}[2026-04-12 14:07:16.612] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 3/3 — 32 chars"}

[2026-04-12 14:07:16.622] ollama_local3090 -> web_interface: {"type":"ollama_response","llm":"local3090","reply_to":"web_interface","msg":"Hello! How can I help you today?"}
[2026-04-12 14:07:16.624] cpu -> web_interface: {"type":"llm_output","correlation_id":1776017057176,"msg":"Hello! How can I help you today?"}
[2026-04-12 14:09:25.792] web_interface -> ollama_local3090: {"type":"chat_request","prompt":"list out your tools","correlation_id":1776017365792}
[2026-04-12 14:09:25.805] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"list out your tools\",\"correlation_id\":1776017365792}'"}
[2026-04-12 14:09:25.816] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 14:09:25.829] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:27b' is available in Ollama ✅"}
[2026-04-12 14:10:55.842] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat))"}
[2026-04-12 14:10:55.842] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:12:28.367] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 2/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat))"}
[2026-04-12 14:12:28.367] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 2/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 14:14:00.892] ollama -> web_interface: {"type":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}[2026-04-12 14:14:00.893] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 3/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}

[2026-04-12 14:14:00.906] logger -> web_interface: {"type":"log","level":"error","msg":"Ollama failed after 3 attempt(s): request timed out after 90s: error sending request for url (http://192.168.1.196:11434/api/chat)"}
[2026-04-12 15:48:28.624] test -> ollama: Test message
[2026-04-12 15:48:28.625] ollama -> web_interface: {"type":"error","msg":"test error — should not panic"}[2026-04-12 15:48:28.625] gemini -> web_interface: {"type":"error","msg":"GEMINI_API_KEY environment variable is not set or empty"}

[2026-04-12 16:32:50.729] logger -> web_interface: {"type":"log","level":"info","msg":"Starting bot with HTTPS web interface and Ollama chat"}
[2026-04-12 16:32:50.730] logger -> web_interface: {"type":"log","level":"info","msg":"HTTPS Web Server spawned - visit https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 16:32:50.731] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'server' spawned on bus channel 'ollama_server'"}
[2026-04-12 16:32:50.732] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama handler 'local3090' spawned on bus channel 'ollama_local3090'"}
[2026-04-12 16:32:50.733] logger -> web_interface: {"type":"log","level":"info","msg":"Gemini handler spawned on bus channel 'gemini'"}
[2026-04-12 16:32:50.734] logger -> web_interface: {"type":"log","level":"info","msg":"Starting HTTPS Web Server on 0.0.0.0:8443"}
[2026-04-12 16:32:50.735] logger -> web_interface: {"type":"log","level":"info","msg":"Web server listening on https://localhost:8443 (accept self-signed cert warning)"}
[2026-04-12 16:32:50.735] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 16:32:50.736] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'server' reachable at http://192.168.1.149:11434 ✅"}
[2026-04-12 16:32:50.742] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.196:11434/api/tags → 200 OK)"}
[2026-04-12 16:32:50.742] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama 'local3090' reachable at http://192.168.1.196:11434 ✅"}
[2026-04-12 16:33:16.078] web_interface -> ollama_server: {"type":"chat_request","prompt":"hi","correlation_id":1776025996078}
[2026-04-12 16:33:16.088] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ← from='web_interface' data='{\"type\":\"chat_request\",\"prompt\":\"hi\",\"correlation_id\":1776025996078}'"}
[2026-04-12 16:33:16.091] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama health-check ✅  (http://192.168.1.149:11434/api/tags → 200 OK)"}
[2026-04-12 16:33:16.094] logger -> web_interface: {"type":"log","level":"info","msg":"Model 'qwen3.5:4b' is available in Ollama ✅"}
[2026-04-12 16:34:46.106] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 1/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat))"}
[2026-04-12 16:34:46.107] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 1/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-04-12 16:36:18.646] ollama -> web_interface: {"type":"warning","msg":"Ollama request failed (attempt 2/3), retrying… (request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat))"}
[2026-04-12 16:36:18.646] logger -> web_interface: {"type":"log","level":"warn","msg":"Ollama attempt 2/3 failed: request timed out after 90s: error sending request for url (http://192.168.1.149:11434/api/chat)"}
[2026-04-12 16:37:25.572] ollama_lan -> cpu: {"type":"llm_response","correlation_id":1776025996078,"msg":"Hello! How can I help you today?"}
[2026-04-12 16:37:25.573] logger -> web_interface: {"type":"log","level":"info","msg":"Ollama ✅  replied on attempt 3/3 — 32 chars"}
[2026-04-12 16:37:25.585] ollama_server -> web_interface: {"type":"ollama_response","llm":"server","reply_to":"web_interface","msg":"Hello! How can I help you today?"}
[2026-04-12 16:37:25.586] cpu -> web_interface: {"type":"llm_output","correlation_id":1776025996078,"msg":"Hello! How can I help you today?"}
[2026-04-13 13:35:03.200] gemini -> web_interface: {"type":"error","msg":"GEMINI_API_KEY environment variable is not set or empty"}[2026-04-13 13:35:03.199] test -> ollama: Test message[2026-04-13 13:35:03.200] ollama -> web_interface: {"type":"error","msg":"test error — should not panic"}


[2026-04-13 13:35:31.688] test -> ollama: Test message[2026-04-13 13:35:31.688] gemini -> web_interface: {"type":"error","msg":"GEMINI_API_KEY environment variable is not set or empty"}

[2026-04-13 13:35:31.689] ollama -> web_interface: {"type":"error","msg":"test error — should not panic"}
[2026-04-13 13:37:44.709] gemini -> web_interface: {"type":"error","msg":"GEMINI_API_KEY environment variable is not set or empty"}[2026-04-13 13:37:44.709] test -> ollama: Test message

[2026-04-13 13:37:44.710] ollama -> web_interface: {"type":"error","msg":"test error — should not panic"}
[2026-05-04 21:16:20.256] web_interface -> ollama_server: {"correlation_id":1777943780256,"prompt":"hi","type":"chat_request"}
[2026-05-04 21:34:32.153] web_interface -> ollama_local3090: {"correlation_id":1777944872153,"prompt":"hi","type":"chat_request"}
[2026-05-04 21:34:37.233] web_interface -> ollama_local3090: {"correlation_id":1777944877232,"prompt":"hi","type":"chat_request"}
