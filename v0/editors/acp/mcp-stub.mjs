// A one-tool MCP server over stdio (newline-delimited JSON-RPC): stands in
// for v0. `replace` edits a file and logs the call, so the client can see it.
import fs from "node:fs"; import readline from "node:readline";
const log = (m) => fs.appendFileSync(process.env.STUB_LOG, JSON.stringify(m) + "\n");
const send = (m) => process.stdout.write(JSON.stringify(m) + "\n");
readline.createInterface({ input: process.stdin }).on("line", (l) => {
  const m = JSON.parse(l);
  if (m.method === "initialize") send({ jsonrpc: "2.0", id: m.id, result: { protocolVersion: m.params.protocolVersion, capabilities: { tools: {} }, serverInfo: { name: "v0", version: "0" }, instructions: "This repository is under v0. Edit files only through this server's tools: the built-in edit, write and shell tools are disabled here on purpose. To change a file, call `replace` with the exact old text, the new text, and the reason for the change." } });
  else if (m.method === "tools/list") send({ jsonrpc: "2.0", id: m.id, result: { tools: [{ name: "replace", description: "Replace text in a file. The only way to edit files.", inputSchema: { type: "object", properties: { path: { type: "string" }, old: { type: "string" }, new: { type: "string" }, reason: { type: "string" } }, required: ["path", "old", "new", "reason"] } }] } });
  else if (m.method === "tools/call") { log(m.params); const a = m.params.arguments; const t = fs.readFileSync(a.path, "utf8"); fs.writeFileSync(a.path, t.replace(a.old, a.new)); send({ jsonrpc: "2.0", id: m.id, result: { content: [{ type: "text", text: "ok" }] } }); }
  else if (m.id !== undefined) send({ jsonrpc: "2.0", id: m.id, result: {} });
});
