# Project Layout and Code Flow

## Overview
This document outlines the structure and code flow of the bot project, providing a clear understanding of how different components interact and the paths data takes through the system.

## Directory Structure
```
/bot
├── .zed/
│   └── task_list.json          # Task tracking and project roadmap
├── Doc's/
│   └── ...                     # Documentation files
├── src/
│   ├── main.rs                 # Entry point of the application
│   ├── hooks/                  # Event-driven scripts or functions triggered by Ollama
│   │   └── alert_on_error.rs   # Example hook for error notifications
│   ├── tooling/                # Utility tools and scripts invoked by Ollama
│   │   └── read_log.rs         # Example tool for log analysis
│   └── memory/                 # Storage for Ollama's conversational and operational memory
│       └── memory_store.json   # Persistent memory data
├── .gitignore                  # Git ignore file for excluding sensitive files and build artifacts
├── Cargo.toml                  # Rust project manifest with dependencies
└── PROJECT_LAYOUT.md           # This file
```

## Code Flow and Data Path

### 1. Application Startup (src/main.rs)
- **Initialization**: The application starts by initializing configurations, logging systems, and necessary directories.
- **Dependencies**: Reads from `Cargo.toml` for library dependencies and project metadata.

### 2. Core Heartbeat Logic
- **Heartbeat Generation**: A periodic signal or data structure is created in `src/main.rs` to represent system status or logs.
- **Error Handling**: Network calls for heartbeat transmission include robust error checking and retry mechanisms.
- **Flow**: `main.rs` → Generates heartbeat data → Prepares for transmission to Ollama

### 3. Ollama Integration
- **Connection**: Establishes a connection to Ollama instance at `192.168.1.149`.
- **Data Transmission**: Sends heartbeat data to Ollama for processing.
- **Response Handling**: Receives and processes responses from Ollama, which may include commands or suggestions.
- **Security**: API keys and settings are securely stored in `.env` (not committed to git).
- **Flow**: `main.rs` → Sends data to Ollama → Receives response → Processes commands or data

### 4. Web Interface (Future Implementation)
- **Server**: A Rust-based HTTPS server will serve a frontend for chat and log display.
- **Interaction**: Users interact via a chat interface, sending messages to Ollama through the backend.
- **Logs Display**: Project logs are fetched and displayed in a dedicated section.
- **Flow**: User → Web Interface → Backend → Ollama → Backend → Web Interface → User

### 5. Terminal Interface (Future Implementation)
- **CLI**: A command-line interface for direct chat with Ollama.
- **Flow**: User → Terminal Input → Application → Ollama → Application → Terminal Output → User

### 6. Logging System
- **Capture**: Events, errors, and interactions are logged throughout the application.
- **Persistence**: Logs are stored persistently for analysis by Ollama.
- **Flow**: Application Events → Logging System → Persistent Storage → Accessible by Ollama or Interfaces

### 7. Tooling Integration
- **Invocation**: Ollama can invoke tools from the `tooling/` directory for specific tasks (e.g., log reading).
- **Execution**: Tools perform actions and return results to Ollama.
- **Flow**: Ollama → Invokes Tool (e.g., `read_log.rs`) → Tool Executes → Returns Data to Ollama

### 8. Hooks Implementation
- **Triggering**: Ollama triggers hooks in `hooks/` based on events or analysis (e.g., error alerts).
- **Action**: Hooks execute predefined actions like notifications or service restarts.
- **Flow**: Event/Analysis → Ollama → Triggers Hook (e.g., `alert_on_error.rs`) → Hook Action

### 9. Memory Management
- **Storage**: Conversational and operational data from Ollama is stored in `memory/`.
- **Types**: Includes short-term (session) and long-term (persistent) memory.
- **Flow**: Ollama Interaction → Data Stored in Memory → Retrieved for Context in Future Interactions

### 10. Error Handling and Network Resilience
- **Across System**: All network calls have timeouts, retries with exponential backoff, and error logging.
- **Critical**: Essential due to Starlink satellite connection variability.
- **Flow**: Network Call → Error Detection → Retry Mechanism → Log Error if Unresolved

## Summary
The bot project's code flow starts from the main application, generating heartbeat data to feed Ollama. Ollama processes this data and can trigger tools or hooks, store memory, or respond through web/terminal interfaces. Logging captures all significant events, and robust error handling ensures resilience against network issues. This layout ensures modularity and clear data paths for future expansions or debugging.
