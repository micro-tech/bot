[2026-06-06 12:47:11.217] web_interface -> ollama_server: {"correlation_id":1780764431217,"prompt":"hi","type":"chat_request"}
[2026-06-06 12:47:38.664] ollama_lan -> cpu: {"correlation_id":1780764431217,"msg":"Hello! 👋 How can I assist you today?","type":"llm_response"}
[2026-06-06 12:47:38.666] ollama_lan -> web_interface: {"correlation_id":1780764431217,"msg":"Hello! 👋 How can I assist you today?","type":"llm_output"}
[2026-06-06 12:47:38.668] ollama_server -> web_interface: {"llm":"server","msg":"Hello! 👋 How can I assist you today?","reply_to":"web_interface","type":"ollama_response"}
[2026-06-08 00:27:45.124] test -> ollama: hello[2026-06-08 00:27:45.124] ollama -> web_interface: {"msg":"test error — should not panic","type":"error"}

[2026-06-08 00:27:45.124] gemini -> web_interface: {"msg":"GEMINI_API_KEY environment variable is not set or empty","type":"error"}
