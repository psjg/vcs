-- Alice, in Neovim, hosting. Driven by v0/tests/editors.rs; see there.
local dir = os.getenv("V0_TEST_DIR")
vim.g.v0_bin = os.getenv("V0_BIN")
vim.opt.runtimepath:append(os.getenv("V0_EDITORS") .. "/nvim")
local live = require("v0live")

local function text() return table.concat(vim.api.nvim_buf_get_lines(0, 0, -1, false), "\n") .. "\n" end
local function exists(name) return vim.uv.fs_stat(dir .. "/" .. name) ~= nil end
local function mark(name, body) local f = assert(io.open(dir .. "/" .. name, "w")) f:write(body or "") f:close() end
local function await(what, cond)
  if not vim.wait(15000, cond, 10) then
    mark("alice.failed", "waiting for " .. what .. "; buffer:\n" .. text() .. "\nerror: " .. tostring((live.state() or {}).error))
    vim.cmd("cquit")
  end
end

vim.cmd("edit " .. dir .. "/alice/demo.rs")
vim.api.nvim_buf_set_lines(0, 0, -1, false, { "fn foo() {}", "fn main() {", "    foo();", "}" })
vim.cmd("V0Live " .. dir .. "/s")
await("ready", function() return live.state() and live.state().ready == "host" end)

-- Live, both ways.
await("bob's line", function() return text():find("// bob here", 1, true) end)
vim.api.nvim_buf_set_lines(0, -1, -1, false, { "// alice hère" })

-- Offline: rename foo from its call.
await("bob to go offline", function() return exists("bob.held") end)
vim.cmd("V0Hold")
vim.api.nvim_win_set_cursor(0, { 3, 5 })
vim.cmd("V0Rename bar")
await("the rename in her own buffer", function() return text():find("fn bar() {}\nfn main() {\n    bar();", 1, true) end)
mark("alice.renamed", text())
await("bob to type", function() return exists("bob.typed") end)
vim.cmd("V0Release")

await("bob's call renamed", function() return text():find("    bar();\n    bar();", 1, true) end)
mark("alice.out", text())
await("bob to finish", function() return exists("bob.out") end)
vim.cmd("qall!")
