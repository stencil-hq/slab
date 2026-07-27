// test: tokenizes every examples/*.slab with the TextMate grammar and asserts key scopes.
// Run from editors/vscode: bun scripts/check-grammar.mjs
import { readFileSync, readdirSync } from "node:fs";
import { createRequire } from "node:module";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath } from "node:url";
import * as oniguruma from "vscode-oniguruma";
import * as textmate from "vscode-textmate";

const here = dirname(fileURLToPath(import.meta.url));
const require = createRequire(import.meta.url);
const wasmPath = join(dirname(require.resolve("vscode-oniguruma")), "onig.wasm");
const grammarPath = resolve(here, "../syntaxes/slab.tmLanguage.json");
const examplesDir = resolve(here, "../../../examples");

await oniguruma.loadWASM(readFileSync(wasmPath).buffer);

const registry = new textmate.Registry({
  onigLib: Promise.resolve({
    createOnigScanner: (patterns) => new oniguruma.OnigScanner(patterns),
    createOnigString: (s) => new oniguruma.OnigString(s),
  }),
  loadGrammar: async (scopeName) => {
    if (scopeName !== "source.slab") return null;
    return textmate.parseRawGrammar(readFileSync(grammarPath, "utf8"), grammarPath);
  },
});

const grammar = await registry.loadGrammar("source.slab");
if (!grammar) throw new Error("failed to load source.slab grammar");

// seen: corpus-wide flags proving each contract-critical scope fired at least once.
const seen = {
  tagRow: false,
  tagCol: false,
  component: false,
  nodeId: false,
  attributeName: false,
  tokenName: false,
  hexColor: false,
  keywordControl: false,
  flag: false,
  reference: false,
  valueKeyword: false,
  colorFunction: false,
  percent: false,
  escape: false,
};

let files = 0;
let lines = 0;
let failures = 0;

for (const name of readdirSync(examplesDir).filter((f) => f.endsWith(".slab")).sort()) {
  files += 1;
  const text = readFileSync(join(examplesDir, name), "utf8");
  let ruleStack = textmate.INITIAL;
  const fileLines = text.split("\n");
  for (let i = 0; i < fileLines.length; i++) {
    const line = fileLines[i];
    lines += 1;
    let result;
    try {
      result = grammar.tokenizeLine(line, ruleStack, 5000);
    } catch (err) {
      failures += 1;
      console.error(`FAIL ${name}:${i + 1} tokenize threw: ${err}`);
      continue;
    }
    if (result.stoppedEarly) {
      failures += 1;
      console.error(`FAIL ${name}:${i + 1} tokenization timed out`);
      continue;
    }
    ruleStack = result.ruleStack;
    for (const tok of result.tokens) {
      const word = line.slice(tok.startIndex, tok.endIndex);
      const scopes = tok.scopes;
      const trimmed = word.trim();
      if (scopes.some((s) => s.startsWith("entity.name.tag.slab"))) {
        if (trimmed === "row") seen.tagRow = true;
        if (trimmed === "col") seen.tagCol = true;
      }
      if (scopes.some((s) => s.startsWith("support.class.component.slab"))) seen.component = true;
      if (scopes.some((s) => s.startsWith("entity.other.attribute-name.id.slab"))) seen.nodeId = true;
      if (scopes.some((s) => s === "entity.other.attribute-name.slab")) seen.attributeName = true;
      if (scopes.some((s) => s === "entity.other.attribute-name.token.slab")) seen.tokenName = true;
      if (scopes.some((s) => s.startsWith("constant.other.color.slab"))) seen.hexColor = true;
      if (scopes.some((s) => s.startsWith("keyword.control"))) seen.keywordControl = true;
      if (scopes.some((s) => s.startsWith("constant.language.flag.slab"))) seen.flag = true;
      if (scopes.some((s) => s.startsWith("variable.other.constant"))) seen.reference = true;
      if (scopes.some((s) => s === "constant.language.slab")) seen.valueKeyword = true;
      if (scopes.some((s) => s.startsWith("support.function.color.slab"))) seen.colorFunction = true;
      if (scopes.some((s) => s.startsWith("constant.numeric.percentage.slab"))) seen.percent = true;
      if (scopes.some((s) => s.startsWith("constant.character.escape.slab"))) seen.escape = true;
    }
  }
}

// Targeted positional assertions on a synthetic line: id vs hex disambiguation.
const probes = [
  { line: "col#card w=360 bg=#0e1116 clip {", word: "#card", scope: "entity.other.attribute-name.id.slab" },
  { line: "col#card w=360 bg=#0e1116 clip {", word: "#0e1116", scope: "constant.other.color.slab" },
  { line: "col #card bg=color.bg {", word: "#card", scope: "entity.other.attribute-name.id.slab" },
  { line: "row gap=8 { Button#save label=\"Go\" }", word: "#save", scope: "entity.other.attribute-name.id.slab" },
  { line: "row gap=8 { Button#save label=\"Go\" }", word: "Button", scope: "support.class.component.slab" },
  { line: "when w<420 { gap=4 }", word: "<", scope: "keyword.operator.comparison.slab" },
  { line: "text label w=fill:2 ellipsis", word: "fill", scope: "constant.language.fill.slab" },
  // Host surface: params/lists, export, each, hole, editing signals/flags, gpu class
  { line: "params {", word: "params", scope: "keyword.control.slab" },
  { line: "def Track(no, title) export {", word: "export", scope: "keyword.control.slab" },
  { line: "hole rows w=fill h=336 scroll", word: "hole", scope: "entity.name.tag.slab" },
  { line: "row focusable act=save pad=8 {", word: "act", scope: "entity.other.attribute-name.binding.slab" },
  { line: "text#field param.draft field=draft w=300", word: "field=", scope: "entity.other.attribute-name.binding.slab" },
  { line: "params { tracks list(Track) = [] }", word: "list", scope: "keyword.control.slab" },
  { line: "each param.tracks", word: "each", scope: "keyword.control.slab" },
  { line: "text param.draft field=draft submit=send multiline", word: "submit=", scope: "entity.other.attribute-name.binding.slab" },
  { line: "text param.draft field=draft submit=send multiline", word: "multiline", scope: "constant.language.flag.slab" },
  { line: "when gpu { tokens { } }", word: "gpu", scope: "support.constant.state.slab" },
  { line: "when dark { tokens { } }", word: "dark", scope: "support.constant.state.slab" },
];
for (const probe of probes) {
  const { tokens } = grammar.tokenizeLine(probe.line, textmate.INITIAL, 5000);
  // Captures may split a word (e.g. `#` + `card`), so assert scope on any token
  // overlapping the word's character range instead of exact token text.
  const start = probe.line.indexOf(probe.word);
  const end = start + probe.word.length;
  const hit = tokens.some(
    (t) =>
      t.startIndex < end &&
      t.endIndex > start &&
      t.scopes.some((s) => s.startsWith(probe.scope))
  );
  if (!hit) {
    failures += 1;
    console.error(`FAIL probe: "${probe.word}" in "${probe.line}" lacks scope ${probe.scope}`);
  }
}

const missing = Object.entries(seen).filter(([, v]) => !v).map(([k]) => k);
if (missing.length > 0) {
  failures += 1;
  console.error(`FAIL corpus coverage missing: ${missing.join(", ")}`);
}

console.log(`checked ${files} files, ${lines} lines`);
console.log(`corpus scope coverage: ${Object.keys(seen).length - missing.length}/${Object.keys(seen).length} scopes seen`);
if (failures > 0) {
  console.error(`${failures} failure(s)`);
  process.exit(1);
}
console.log("grammar check PASSED");
