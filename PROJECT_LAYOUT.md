# Project Layout and Code Flow

## Overview
This document outlines the structure and code flow of the bot project, conceptualized as an **Agent OS**—an operating system designed to operate under the control of an agent or Large Language Model (LLM). The system facilitates autonomous operations, data processing, and interaction through various components. This layout provides a clear understanding of how different components interact, the paths data takes through the system, and highlights areas that are yet to be built.

## Directory Structure
```
/bot
├── .zed/
│   └── task_list.json          # Task tracking and project roadmap
├── Doc's/
│   └── ...                     # Documentation files
├── logs/
│   ├── error_log.md            # Log file for errors
│   ├── chat_log.md             # Log file for chat interactions
│   ├── hartbeat_log.md         # Log file for heartbeat-specific data
│   └── bus_log.md              # Log file for bus system transactions
├── src/
│   ├── main.rs                 # Entry point of the application (Agent OS Kernel)
│   ├── cpu/                    # Central Processing Unit for agent state management and instruction execution
│   │   ├── mod.rs              # CPU module definition and main Cpu struct
│   │   ├── state.rs            # Agent state management (mode, step counter, etc.)
│   │   ├── scheduler.rs        # Instruction scheduling based on state and events
│   │   ├── executor.rs         # Execution of scheduled instructions
│   │   ├── interrupts.rs       # Event polling and interrupt handling from bus
│   │   └── instructions.rs     # Definitions of CPU instructions and events
│   ├── bus/                    # Central communication bus for inter-component messaging
│   │   ├── bus.rs              # Core bus implementation for message routing
│   │   └── message.rs          # Message structure and handling logic
│   ├── a2a/                    # Agent-to-Agent communication module (TBD)
│   │   └── a2a_handler.rs      # Placeholder for A2A communication logic
│   ├── cron/                   # Scheduled tasks and automation (TBD)
│   │   └── cron_handler.rs     # Placeholder for scheduling logic
│   ├── hartbeat/               # Core heartbeat logic for system status signals
│   │   └── hartbeat.rs         # Implementation of periodic data generation
│   ├── io/                     # Input/Output handling for interfaces
│   │   ├── io.rs               # Central IO manager for routing bus messages
│   │   ├── io_test.rs          # Tests for IO manager functions
│   │   ├── https_web/          # Secure web interface for chat and logs (partial TBD)
│   │   │   ├── web_server.rs   # HTTPS server implementation
│   │   │   └── web_server_test.rs  # Tests for web server functions
│   │   ├── terminal/           # Command-line interface for direct interaction (partial TBD)
│   │   │   ├── cli.rs          # Terminal input/output logic
│   │   │   └── cli_test.rs     # Tests for terminal functions
│   │   ├── ollama/             # Ollama communication handler
│   │   │   ├── ollama_handler.rs  # Handles messages to/from Ollama via bus
│   │   │   └── ollama_handler_test.rs  # Tests for Ollama handler functions
│   │   └── llm_gemini/         # Placeholder for future Gemini LLM integration
│   │       ├── gemini_handler.rs  # Placeholder for Gemini API handling
│   │       └── gemini_handler_test.rs  # Tests for Gemini handler functions
│   ├── mcp/                    # Master Control Program for overseeing agent operations (TBD)
│   │   └── mcp_handler.rs      # Placeholder for central control logic
│   ├── hooks/                  # Event-driven scripts or functions triggered by Ollama
│   │   └── alert_on_error.rs   # Example hook for error notifications
│   ├── tooling/                # Utility tools and scripts invoked by Ollama or Agent OS
│   │   └── read_log.rs         # Example tool for log analysis
│   └── memory/                 # Storage for Ollama's conversational and operational memory
│       ├── short_term/         # Short-term memory for current session context (TBD)
│       │   └── session_data.json # Placeholder for session-specific data
│       └── long_term/          # Long-term memory for persistent data across restarts (TBD)
│           └── memory_store.json # Persistent memory data
├── certs/                      # Directory for SSL certificates (TBD)
│   ├── server.crt              # SSL certificate for HTTPS web interface
│   └── server.key              # SSL private key for HTTPS web interface
├── .gitignore                  # Git ignore file for excluding sensitive files and build artifacts
├── Cargo.toml                  # Rust project manifest with dependencies
├── config.toml                 # Configuration file for Agent OS settings (Ollama, logs, ports, bus, etc.)
└── PROJECT_LAYOUT.md           # This file
```

## Conceptual Framework: Agent OS
The bot project is designed as an **Agent OS**, a specialized operating system that operates under the direction of an LLM or agent (e.g., Ollama). The OS manages resources, schedules tasks, handles communications, and provides interfaces for interaction, much like a traditional OS, but with a focus on agent-driven automation and intelligence. A central **Bus System** enables decoupled communication between components, allowing them to publish and subscribe to messages. The components are modular, allowing for scalability and future development. Many components are marked as 'To Be Developed' (TBD), indicating areas for future implementation. Settings for components like Ollama connection details, log file locations, bus configurations, and port settings are centralized in `config.toml` to avoid hardcoding in source files.

## Code Flow and Data Path

### 1. Application Startup (src/main.rs) - Agent OS Kernel
- **Initialization**: The application starts by initializing configurations, logging systems, and necessary directories. This acts as the kernel of the Agent OS, coordinating all subsystems.
- **Configuration**: Reads settings from `config.toml` for Ollama connection, log locations, port settings, bus configurations, and other runtime parameters.
- **Bus Initialization**: Initializes the central bus system (`src/bus/`) to handle inter-component communication.
- **Dependencies**: Reads from `Cargo.toml` for library dependencies and project metadata.
- **Flow**: `main.rs` → Reads `config.toml` → Initializes Agent OS → Boots subsystems (`hartbeat`, `io`, `bus`, etc.)

### 2. CPU Execution Loop (src/cpu/) - Agent State Management and Instruction Processing
- **Purpose**: The CPU module manages the agent's state, polls for events, schedules instructions, and executes them in a continuous loop.
- **Components**:
  - **`state.rs`**: Tracks agent mode (Idle, Conversational, etc.), step counter, and other state variables.
  - **`interrupts.rs`**: Polls the bus for incoming events and converts them to CPU events.
  - **`scheduler.rs`**: Decides which instructions to execute based on current state and events.
  - **`executor.rs`**: Executes instructions, interacting with memory, skills, hooks, and the bus.
  - **`instructions.rs`**: Defines the types of instructions (e.g., RunSkill, WriteMemory) and events.
  - **`mod.rs`**: Orchestrates the CPU loop, calling interrupts, scheduler, and executor.
- **Logging**: All CPU activities (polling, scheduling, execution) are logged to `logs/error_log.md` with timestamps.
- **Flow**: CPU Loop → Polls for events via `interrupts.rs` → `scheduler.rs` schedules instructions → `executor.rs` executes them → Updates state in `state.rs` → Loop repeats

### 3. Bus System (src/bus/) - Central Communication Framework
- **Purpose**: A central messaging bus for inter-component communication, allowing subsystems to publish and subscribe to messages in a decoupled manner.
- **Message Format**: Messages include `to` (destination component), `from` (source component), and `data` (payload), defined in `message.rs`.
- **Routing**: The bus (`bus.rs`) routes messages to appropriate subscribers based on the `to` field.
- **Logging**: All bus transactions are logged with timestamps to `logs/bus_log.md` as specified in `config.toml`.
- **Flow**: Component → Publishes message to bus (`to: destination, from: source, data: payload`) → Bus logs transaction → Routes to subscriber → Subscriber processes data

### 4. Core Heartbeat Logic (src/hartbeat/)
- **Heartbeat Generation**: A periodic signal or data structure is created in `src/hartbeat/hartbeat.rs` to represent system status or logs, with the interval defined in `config.toml`.
- **Bus Interaction**: Reads data from `hartbeat_log.md`, marks it for Ollama, and publishes a message to the bus with `to: ollama`, `from: hartbeat`, and `data: heartbeat payload`.
- **Error Handling**: Network calls for heartbeat transmission include robust error checking and retry mechanisms, with retry settings from `config.toml`.
- **Logging**: Heartbeat-specific logs are written to the location specified in `config.toml` (`logs/hartbeat_log.md`).
- **Flow**: `hartbeat.rs` → Generates heartbeat data → Logs to `hartbeat_log.md` → Publishes to bus (`to: ollama`) → Bus logs transaction

### 5. Ollama Integration (src/io/ollama/)
- **Connection**: Establishes a connection to Ollama instance using host and port settings from `config.toml` (e.g., `192.168.1.149:11434`).
- **Model Selection**: Uses the specified model (e.g., `llama3`) from `config.toml` for LLM tasks.
- **Bus Subscription**: Subscribes to bus messages with `to: ollama` via `ollama_handler.rs`, processes incoming data (e.g., from `hartbeat`), and sends it to Ollama API.
- **Response Handling**: Receives responses from Ollama and publishes them back to the bus if necessary (e.g., `to: hartbeat` or `to: web_interface`).
- **Security**: API keys and settings are securely stored in `.env` or `config.toml` (not committed to git).
- **Flow**: `ollama_handler.rs` → Subscribes to bus (`to: ollama`) → Receives data → Sends to Ollama → Receives response → Publishes response to bus if needed

### 6. Agent-to-Agent Communication (src/a2a/) - TBD
- **Purpose**: Facilitates communication between multiple agents or LLMs for collaborative tasks.
- **Status**: To be developed. This module will handle protocols for agent interaction.
- **Bus Integration**: Will publish/subscribe to bus messages for A2A communication.
- **Configuration**: Future settings for A2A communication protocols will be added to `config.toml`.
- **Flow**: Agent OS → `a2a` → Publishes/subscribes to bus → Communicates with other agents → Returns data or commands

### 7. Scheduled Tasks (src/cron/) - TBD
- **Purpose**: Manages scheduled tasks and automation, allowing the Agent OS to perform routine operations.
- **Status**: To be developed. This will include cron-like job scheduling for the agent.
- **Bus Integration**: Will publish task results or triggers to the bus for other components.
- **Configuration**: Future cron job schedules and settings will be defined in `config.toml`.
- **Flow**: Agent OS → `cron` → Executes scheduled tasks → Publishes results to bus → Logs results or triggers hooks

### 8. Input/Output Interfaces (src/io/)
- **Web Interface (src/io/https_web/)**:
  - **Server**: A Rust-based HTTPS server serves a frontend for chat and log display, using the port and certificate paths specified in `config.toml`.
  - **Interaction**: Users interact via a chat interface, sending messages to Ollama through the backend by publishing to the bus (`to: ollama`).
  - **Bus Subscription**: Subscribes to bus messages (e.g., `to: web_interface`) for Ollama responses or log updates to display.
  - **Logs Display**: Project logs are fetched and displayed in a dedicated section.
  - **Status**: Partially implemented; full frontend integration TBD.
  - **Flow**: User → Web Interface → Publishes to bus (`to: ollama`) → Ollama processes → Publishes response to bus (`to: web_interface`) → Web Interface displays to user
- **Terminal Interface (src/io/terminal/)**:
  - **CLI**: A command-line interface for direct chat with Ollama, using a custom prompt defined in `config.toml`.
  - **Bus Interaction**: Publishes user input to bus (`to: ollama`) and subscribes to responses (`to: terminal`).
  - **Status**: Partially implemented; full error handling TBD.
  - **Flow**: User → Terminal Input → Publishes to bus (`to: ollama`) → Ollama processes → Publishes to bus (`to: terminal`) → Terminal Output to user

### 9. Master Control Program (src/mcp/) - TBD
- **Purpose**: Oversees all agent operations, acting as a central decision-making module for the Agent OS.
- **Status**: To be developed. This will manage resource allocation and task prioritization.
- **Bus Integration**: Will monitor bus traffic and issue commands via the bus to other components.
- **Configuration**: Future MCP settings will be added to `config.toml`.
- **Flow**: Agent OS → `mcp` → Monitors bus → Publishes commands to bus → Coordinates subsystems

### 10. Logging System (logs/)
- **Capture**: Events, errors, and interactions are logged throughout the application in locations specified in `config.toml` (`logs/error_log.md`, `logs/chat_log.md`, `logs/hartbeat_log.md`).
- **Bus Logging**: Bus transactions are logged to `logs/bus_log.md` with timestamps, including `to`, `from`, and a summary of `data`.
- **Persistence**: Logs are stored persistently for analysis by Ollama.
- **Flow**: Application Events → Logging System → Persistent Storage → Accessible by Ollama or Interfaces; Bus Transaction → Logged to `bus_log.md`

### 11. Tooling Integration (src/tooling/)
- **Invocation**: Ollama or the Agent OS can invoke tools from the `tooling/` directory for specific tasks (e.g., log reading) by publishing requests to the bus.
- **Execution**: Tools subscribe to relevant bus messages, perform actions, and publish results back to the bus.
- **Status**: Initial tools like `read_log.rs` implemented; more complex tools TBD.
- **Flow**: Ollama/Agent OS → Publishes tool request to bus (`to: tooling`) → Tool subscribes and executes → Publishes result to bus

### 12. Hooks Implementation (src/hooks/)
- **Triggering**: Ollama triggers hooks in `hooks/` based on events or analysis (e.g., error alerts) by publishing to the bus.
- **Action**: Hooks subscribe to relevant messages and execute predefined actions like notifications or service restarts.
- **Flow**: Event/Analysis → Ollama → Publishes to bus (`to: hooks`) → Hook subscribes and executes action

### 13. Memory Management (src/memory/)
- **Storage**: Conversational and operational data from Ollama is stored in `memory/`, with file paths specified in `config.toml`.
- **Types**: 
  - **Short-term (src/memory/short_term/)**: For current session context, stored in `session_data.json`. Status: TBD.
  - **Long-term (src/memory/long_term/)**: For persistent data across restarts, stored in `memory_store.json`. Status: TBD.
- **Bus Integration**: Memory updates or retrievals can be requested via bus messages.
- **Flow**: Ollama Interaction → Publishes memory update to bus → Memory module updates storage → Retrieved for context in future interactions via bus

### 14. Error Handling and Network Resilience
- **Across System**: All network calls have timeouts, retries with exponential backoff, and error logging, with settings defined in `config.toml`.
- **Critical**: Essential due to Starlink satellite connection variability.
- **Bus Error Handling**: Errors in bus communication are logged to `bus_log.md` and `error_log.md`.
- **Flow**: Network Call → Error Detection → Retry Mechanism → Log Error if Unresolved → Publish error report to bus if critical

## Summary
The bot project, conceptualized as an **Agent OS**, starts from the main application (`main.rs`), acting as the kernel to coordinate subsystems like `cpu`, `hartbeat`, `io`, `tooling`, and future components such as `a2a`, `cron`, and `mcp`. The **CPU** (`src/cpu/`) manages agent state and executes instructions in a loop, polling for events via the **Bus System** (`src/bus/`), which facilitates decoupled communication by allowing components to publish and subscribe to messages (e.g., `hartbeat` sends data to `ollama` via the bus, handled by `io/ollama`). The bus logs all transactions to `bus_log.md` for monitoring. Configuration settings for Ollama, log locations, ports, bus parameters, and other settings are centralized in `config.toml` to ensure flexibility and avoid hardcoding. Logging captures all significant events, and robust error handling ensures resilience against network issues. Many components are marked as 'To Be Developed' (TBD), indicating areas for future expansion. This layout ensures modularity and clear data paths for future development or debugging by other AI or developers.
