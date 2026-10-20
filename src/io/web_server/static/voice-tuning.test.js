// Task 275: unit tests for the voice-tuning pure helpers (push-to-talk
// hotkey matching + autoplay gating). The functions under test are extracted
// verbatim from index.html (between the VOICE-275-PURE markers) so the tests
// always run against the shipped code.
//
// Run: node src/io/web_server/static/voice-tuning.test.js
"use strict";

const fs = require("fs");
const path = require("path");
const assert = require("assert");

const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
const m = html.match(
    /\/\/ ==== VOICE-275-PURE-START ====([\s\S]*?)\/\/ ==== VOICE-275-PURE-END ====/
);
assert.ok(m, "pure-function block markers not found in index.html");
const { voicePttKeyMatches, voiceShouldAutoplay } = new Function(
    m[1] + "\nreturn { voicePttKeyMatches, voiceShouldAutoplay };"
)();
assert.strictEqual(
    typeof voicePttKeyMatches,
    "function",
    "voicePttKeyMatches missing"
);
assert.strictEqual(
    typeof voiceShouldAutoplay,
    "function",
    "voiceShouldAutoplay missing"
);

let n = 0;
function check(name, fn) {
    fn();
    n++;
    console.log("ok - " + name);
}

check("ptt: matching code activates", () => {
    assert.strictEqual(
        voicePttKeyMatches("ControlRight", "ControlRight"),
        true
    );
});

check("ptt: different code ignored", () => {
    assert.strictEqual(
        voicePttKeyMatches("ControlLeft", "ControlRight"),
        false
    );
    assert.strictEqual(voicePttKeyMatches("Space", "ControlRight"), false);
});

check("ptt: empty configured value disables hotkey", () => {
    assert.strictEqual(voicePttKeyMatches("ControlRight", ""), false);
    assert.strictEqual(voicePttKeyMatches("KeyV", ""), false);
});

check("ptt: custom binding respected", () => {
    assert.strictEqual(voicePttKeyMatches("KeyV", "KeyV"), true);
    assert.strictEqual(voicePttKeyMatches("ControlRight", "KeyV"), false);
});

check("autoplay: agent reply plays when toggle is on", () => {
    assert.strictEqual(
        voiceShouldAutoplay("agent_msg", false, false, true, false),
        true
    );
});

check("autoplay: silent when toggle is off", () => {
    assert.strictEqual(
        voiceShouldAutoplay("agent_msg", false, false, false, false),
        false
    );
});

check("autoplay: never user messages", () => {
    assert.strictEqual(
        voiceShouldAutoplay("user_msg", true, false, true, false),
        false
    );
});

check("autoplay: never errors or warnings", () => {
    assert.strictEqual(
        voiceShouldAutoplay("error", false, true, true, false),
        false
    );
    assert.strictEqual(
        voiceShouldAutoplay("warning", false, false, true, false),
        false
    );
});

check("autoplay: never while the mic is recording", () => {
    assert.strictEqual(
        voiceShouldAutoplay("agent_msg", false, false, true, true),
        false
    );
});

console.log(n + " tests passed");
