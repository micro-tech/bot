[2026-06-06 12:47:11.217] web_interface -> ollama_server: {"correlation_id":1780764431217,"prompt":"hi","type":"chat_request"}
[2026-06-06 12:47:38.664] ollama_lan -> cpu: {"correlation_id":1780764431217,"msg":"Hello! 👋 How can I assist you today?","type":"llm_response"}
[2026-06-06 12:47:38.666] ollama_lan -> web_interface: {"correlation_id":1780764431217,"msg":"Hello! 👋 How can I assist you today?","type":"llm_output"}
[2026-06-06 12:47:38.668] ollama_server -> web_interface: {"llm":"server","msg":"Hello! 👋 How can I assist you today?","reply_to":"web_interface","type":"ollama_response"}
[2026-06-08 00:27:45.124] test -> ollama: hello[2026-06-08 00:27:45.124] ollama -> web_interface: {"msg":"test error — should not panic","type":"error"}

[2026-06-08 00:27:45.124] gemini -> web_interface: {"msg":"GEMINI_API_KEY environment variable is not set or empty","type":"error"}
[2026-06-11 00:31:26.316] test_harness -> nonexistent: {"type":"ping"}
[2026-06-11 00:31:26.316] harness -> test_harness: {"type":"error","msg":"test failure"}[2026-06-11 00:31:26.316] harness -> ollama_local: {"type":"chat_request","prompt":"hello"}[2026-06-11 00:31:26.316] harness -> test_harness: not valid json {[2026-06-11 00:31:26.316] harness -> test_topic: {"seq":0}



[2026-06-11 00:31:26.321] harness -> test_topic: {"seq":1}
[2026-06-11 00:31:26.321] harness -> test_topic: {"seq":2}
[2026-06-11 00:31:26.322] harness -> test_topic: {"seq":3}
[2026-06-11 00:31:26.322] harness -> test_topic: {"seq":4}
[2026-06-11 00:35:39.343] harness -> test_harness: {"type":"error","msg":"test failure"}
[2026-06-11 00:41:55.568] harness -> ollama_local: {"type":"chat_request","prompt":"hello"}[2026-06-11 00:41:55.568] harness -> test_topic: {"seq":0}
[2026-06-11 00:41:55.568] test_harness -> nonexistent: {"type":"ping"}
[2026-06-11 00:41:55.568] harness -> test_harness: not valid json {[2026-06-11 00:41:55.568] harness -> test_harness: {"type":"error","msg":"test failure"}


[2026-06-11 00:41:55.572] harness -> test_topic: {"seq":1}
[2026-06-11 00:41:55.573] harness -> test_topic: {"seq":2}
[2026-06-11 00:41:55.575] harness -> test_topic: {"seq":3}
[2026-06-11 00:41:55.575] harness -> test_topic: {"seq":4}
[2026-06-11 00:51:44.418] harness -> test_topic: {"seq":0}[2026-06-11 00:51:44.418] harness -> test_harness: {"type":"error","msg":"test failure"}
[2026-06-11 00:51:44.418] harness -> ollama_local: {"type":"chat_request","prompt":"hello"}[2026-06-11 00:51:44.419] harness -> test_harness: not valid json {[2026-06-11 00:51:44.419] test_harness -> nonexistent: {"type":"ping"}



[2026-06-11 00:51:44.427] harness -> test_topic: {"seq":1}
[2026-06-11 00:51:44.428] harness -> test_topic: {"seq":2}
[2026-06-11 00:51:44.429] harness -> test_topic: {"seq":3}
[2026-06-11 00:51:44.429] harness -> test_topic: {"seq":4}
[2026-06-11 01:08:10.868] harness -> test_topic: {"seq":0}[2026-06-11 01:08:10.868] harness -> test_harness: not valid json {[2026-06-11 01:08:10.868] harness -> test_harness: {"type":"error","msg":"test failure"}[2026-06-11 01:08:10.868] test_harness -> nonexistent: {"type":"ping"}[2026-06-11 01:08:10.868] harness -> ollama_local: {"type":"chat_request","prompt":"hello"}




[2026-06-11 01:08:10.871] harness -> test_topic: {"seq":1}
[2026-06-11 01:08:10.872] harness -> test_topic: {"seq":2}
[2026-06-11 01:08:10.873] harness -> test_topic: {"seq":3}
[2026-06-11 01:08:10.873] harness -> test_topic: {"seq":4}
