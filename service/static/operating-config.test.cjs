'use strict';
const test = require('node:test');
const assert = require('node:assert/strict');
const { parseValue, equal } = require('./operating-config.js');
const field = (kind, extra = {}) => ({ kind, nullable: false, min: null, max: null, length: null, choices: [], ...extra });

test('Textgrenzen entsprechen UTF-8-Bytes und Steuerzeichen des Servers', () => {
    const text = field('string');
    assert.equal(parseValue(text, 'ä'.repeat(1024)), 'ä'.repeat(1024));
    assert.throws(() => parseValue(text, 'ä'.repeat(1025)));
    assert.throws(() => parseValue(text, 'ungültig\u0085'));
    assert.equal(parseValue(field('string', { nullable: true }), ''), '');
});
test('Snowflakes bleiben exakte Dezimalstrings, einschließlich oberhalb Number.MAX_SAFE_INTEGER', () => {
    const id = field('integer', { min: '1', max: '9223372036854775807' });
    for (const value of ['1547199955133927464', '9007199254740993', '9223372036854775807']) {
        assert.equal(parseValue(id, value), value);
        assert.equal(JSON.parse(JSON.stringify({ changes: { role: parseValue(id, value) } })).changes.role, value);
    }
    for (const value of ['1e18', '1.5', '-1', '01', '-0', '9223372036854775808']) assert.throws(() => parseValue(id, value));
});
test('Dienststandard, Aus und eine leere Liste sind unterschiedliche Werte', () => {
    const bool = field('boolean', { nullable: true });
    assert.equal(parseValue(bool, null), null); assert.equal(parseValue(bool, false), false); assert.equal(equal(null, false), false);
    assert.deepEqual(parseValue(field('integer_list', { min: '0', max: '100' }), ''), []);
    assert.throws(() => parseValue(field('boolean'), null));
});
test('Feste Listenlängen, Auswahlwerte und KI-Grenzen werden geprüft', () => {
    const list = field('integer_list', { min: '1', max: '9223372036854775807', length: 2 });
    assert.deepEqual(parseValue(list, '1547199955133927464\n1547199955133927465'), ['1547199955133927464', '1547199955133927465']);
    assert.throws(() => parseValue(list, '1'));
    assert.throws(() => parseValue(field('choice', { choices: ['fireworks', 'openai'] }), 'unknown'));
    assert.throws(() => parseValue(field('number', { min: '0', max: '2' }), '3'));
    assert.equal(parseValue(field('number', { min: '0', max: '2' }), '0.4'), 0.4);
});
