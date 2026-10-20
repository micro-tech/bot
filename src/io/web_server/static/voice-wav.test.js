// Task 274: unit tests for the WAV encoder + 16 kHz downsampler used by the
// push-to-talk mic button. The functions under test are extracted verbatim
// from index.html (between the VOICE-CAPTURE-PURE markers) so the tests
// always run against the shipped code.
//
// Run: node src/io/web_server/static/voice-wav.test.js
"use strict";

const fs = require("fs");
const path = require("path");
const assert = require("assert");

const html = fs.readFileSync(path.join(__dirname, "index.html"), "utf8");
const m = html.match(
    /\/\/ ==== VOICE-CAPTURE-PURE-START ====([\s\S]*?)\/\/ ==== VOICE-CAPTURE-PURE-END ====/
);
assert.ok(m, "pure-function block markers not found in index.html");
const { voiceDownsampleTo16k, voiceEncodeWav16 } = new Function(
    m[1] + "\nreturn { voiceDownsampleTo16k, voiceEncodeWav16 };"
)();
assert.strictEqual(
    typeof voiceDownsampleTo16k,
    "function",
    "voiceDownsampleTo16k missing"
);
assert.strictEqual(
    typeof voiceEncodeWav16,
    "function",
    "voiceEncodeWav16 missing"
);

function dv(bytes) {
    return new DataView(bytes.buffer, bytes.byteOffset, bytes.byteLength);
}
function ascii(v, off, len) {
    let s = "";
    for (let i = 0; i < len; i++) s += String.fromCharCode(v.getUint8(off + i));
    return s;
}

let n = 0;
function check(name, fn) {
    fn();
    n++;
    console.log("ok - " + name);
}

check("wav header: RIFF/WAVE magic, fmt = 16-bit mono 16000 Hz", () => {
    const bytes = voiceEncodeWav16(new Float32Array(16000));
    const v = dv(bytes);
    assert.strictEqual(bytes.length, 44 + 32000);
    assert.strictEqual(ascii(v, 0, 4), "RIFF");
    assert.strictEqual(v.getUint32(4, true), 36 + 32000);
    assert.strictEqual(ascii(v, 8, 4), "WAVE");
    assert.strictEqual(ascii(v, 12, 4), "fmt ");
    assert.strictEqual(v.getUint32(16, true), 16); // fmt chunk size
    assert.strictEqual(v.getUint16(20, true), 1); // PCM
    assert.strictEqual(v.getUint16(22, true), 1); // mono
    assert.strictEqual(v.getUint32(24, true), 16000); // sample rate
    assert.strictEqual(v.getUint32(28, true), 32000); // byte rate
    assert.strictEqual(v.getUint16(32, true), 2); // block align
    assert.strictEqual(v.getUint16(34, true), 16); // bits per sample
    assert.strictEqual(ascii(v, 36, 4), "data");
    assert.strictEqual(v.getUint32(40, true), 32000);
    for (let i = 0; i < 16000; i++) {
        assert.strictEqual(v.getInt16(44 + i * 2, true), 0);
    }
});

check("wav encoder clamps to int16 range", () => {
    const bytes = voiceEncodeWav16(
        new Float32Array([1.5, -1.5, 1.0, -1.0, 0.5, 0.0])
    );
    const v = dv(bytes);
    assert.strictEqual(bytes.length, 44 + 12);
    assert.strictEqual(v.getInt16(44, true), 32767);
    assert.strictEqual(v.getInt16(46, true), -32768);
    assert.strictEqual(v.getInt16(48, true), 32767);
    assert.strictEqual(v.getInt16(50, true), -32768);
    assert.strictEqual(v.getInt16(52, true), 16383);
    assert.strictEqual(v.getInt16(54, true), 0);
});

check("wav encoder handles empty input", () => {
    const bytes = voiceEncodeWav16(new Float32Array(0));
    const v = dv(bytes);
    assert.strictEqual(bytes.length, 44);
    assert.strictEqual(v.getUint32(4, true), 36);
    assert.strictEqual(v.getUint32(40, true), 0);
});

check("downsample 48000 -> 16000: 3:1 length, DC preserved", () => {
    const src = new Float32Array(48000).fill(0.25);
    const out = voiceDownsampleTo16k(src, 48000);
    assert.strictEqual(out.length, 16000);
    for (let i = 0; i < out.length; i++) {
        assert.ok(Math.abs(out[i] - 0.25) < 1e-6, "drift at " + i);
    }
});

check("downsample 44100 -> 16000: correct length", () => {
    const out = voiceDownsampleTo16k(new Float32Array(44100), 44100);
    assert.strictEqual(out.length, 16000);
});

check("downsample at 16000 Hz is a copy", () => {
    const src = new Float32Array([0.1, -0.2, 0.3]);
    const out = voiceDownsampleTo16k(src, 16000);
    assert.notStrictEqual(out, src);
    assert.deepStrictEqual(Array.from(out), Array.from(src));
});

check("downsample preserves 440 Hz sine shape", () => {
    const rate = 48000;
    const f = 440;
    const src = new Float32Array(rate);
    for (let i = 0; i < src.length; i++) {
        src[i] = Math.sin((2 * Math.PI * f * i) / rate);
    }
    const out = voiceDownsampleTo16k(src, rate);
    assert.strictEqual(out.length, 16000);
    let err = 0;
    for (let i = 0; i < out.length; i++) {
        err += Math.abs(out[i] - Math.sin((2 * Math.PI * f * i) / 16000));
    }
    assert.ok(err / out.length < 0.01, "mean abs err " + err / out.length);
});

check("downsample handles empty and invalid input", () => {
    assert.strictEqual(
        voiceDownsampleTo16k(new Float32Array(0), 48000).length,
        0
    );
    assert.strictEqual(voiceDownsampleTo16k(new Float32Array(10), 0).length, 0);
});

check("end-to-end: chunk concat + downsample + wav round-trip", () => {
    const mk = (start, len) => {
        const a = new Float32Array(len);
        for (let i = 0; i < len; i++) {
            a[i] = Math.sin((2 * Math.PI * 440 * (start + i)) / 48000);
        }
        return a;
    };
    const chunks = [mk(0, 24000), mk(24000, 24000)];
    let total = 0;
    for (const c of chunks) total += c.length;
    const pcm = new Float32Array(total);
    let off = 0;
    for (const c of chunks) {
        pcm.set(c, off);
        off += c.length;
    }
    const pcm16k = voiceDownsampleTo16k(pcm, 48000);
    assert.strictEqual(pcm16k.length, 16000);
    const wav = voiceEncodeWav16(pcm16k);
    const v = dv(wav);
    assert.strictEqual(ascii(v, 0, 4), "RIFF");
    assert.strictEqual(ascii(v, 8, 4), "WAVE");
    assert.strictEqual(v.getUint32(24, true), 16000);
    for (const i of [0, 100, 1000, 8000, 15999]) {
        const got = v.getInt16(44 + i * 2, true) / 32767;
        const want = Math.sin((2 * Math.PI * 440 * i) / 16000);
        assert.ok(Math.abs(got - want) < 0.02, "sample " + i);
    }
});

console.log("\n" + n + " tests passed");
