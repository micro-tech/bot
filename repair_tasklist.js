const fs = require('fs');
const path = '.zed/task_list.json';
const backup = '.zed/task_list.json.bak';

let content = fs.readFileSync(path, 'utf8');

// Backup first
fs.writeFileSync(backup, content, 'utf8');
console.log('Backup created: ' + backup);

// Clean trailing junk
content = content.trim();
if (content.endsWith('"')) {
  content = content.slice(0, -1).trim();
}

// Ensure last object is properly terminated
const lastBrace = content.lastIndexOf('}');
if (lastBrace > 0) {
  content = content.substring(0, lastBrace + 1);
}

// Add comma if needed before new element
if (!content.endsWith(',')) {
  content += ',';
}

const newTask = `
  {
    "id": 121,
    "title": "Integrate Reasoning Engine + HyEvo + Manifest Evolution (Eliminate Dead Code)",
    "description": "Wire the pre-built reasoning engine, full HyEvo evolutionary system, OllamaLlm implementation, and manifest evolution logic into the main Cpu loop and bus handlers so that the large amount of currently dead_code infrastructure becomes active and exercised.",
    "status": "pending",
    "priority": "high",
    "dependencies": [],
    "details": "Key work: Instantiate ReasoningEngine in Cpu, enable HyEvo by default, connect OllamaLlm, activate manifest evolution, add integration tests.",
    "testStrategy": "cargo check warnings drop significantly; HyEvo and reasoning traces appear in logs.",
    "subtasks": [
      { "id": "121.1", "title": "Wire ReasoningEngine into Cpu", "status": "pending" },
      { "id": "121.2", "title": "Enable HyEvo and connect OllamaLlm", "status": "pending" },
      { "id": "121.3", "title": "Activate manifest evolution paths", "status": "pending" },
      { "id": "121.4", "title": "Add integration tests", "status": "pending" },
      { "id": "121.5", "title": "Reduce dead_code warnings", "status": "pending" }
    ]
  }
]`;

fs.writeFileSync(path, content + newTask, 'utf8');
console.log('Task 121 added and JSON repaired successfully.');
