# Chat Log

All messages put on the bus are logged here with timestamp, sender, receiver, and data summary (truncated to 200 chars).

## Usage
When the bot runs and heartbeat publishes, you'll see entries like:
```
[2024-XX-XX XX:XX:XX] hartbeat -> ollama: {&quot;timestamp&quot;:1234567890,...}
```
[2026-03-06 16:56:16.013] hartbeat -> ollama: {"timestamp":1772834176,"system_status":"Operational","recent_events":["System check completed"]}

