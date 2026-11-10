--- v0 live, for Neovim: this buffer as one replica of a shared document.
---
--- The session (`v0 live`, v0/src/live.rs) owns the document; this file owns
--- nothing but the buffer. It reports what was typed as "replace `del` bytes
--- at `at` with `text`" and applies the same from peers.
---
---   :V0Live /tmp/x.sock   start: host if no one is on the socket, else join
---   :V0Rename bar         rename the name under the cursor, as an intent
---   :V0Hold / :V0Release  go offline / come back
---
--- Both directions go through one string, `shadow`: the document as the
--- session last knew this buffer to show. A local change is whatever differs
--- between the buffer and the shadow, found by trimming the common head and
--- tail; that costs the document's length per keystroke and in return never
--- depends on what a particular command reports about itself.
local api = vim.api
local M = {}

--- The state of the one live buffer; nil when off.
local S = nil

--- Lines as the document's text. Neovim keeps no final newline; `eol` says
--- whether the document has one.
local function text_of(lines, eol)
  return table.concat(lines, "\n") .. (eol and "\n" or "")
end

--- The other way: a text as lines, and whether it ended in a newline.
local function lines_of(text)
  local eol = text:sub(-1) == "\n"
  return vim.split(eol and text:sub(1, -2) or text, "\n", { plain = true }), eol
end

local function continuation(byte)
  return byte and byte >= 0x80 and byte < 0xC0
end

--- What turns `a` into `b`: (at, del, text), in bytes, cut between
--- characters. nil when they are equal.
local function diff(a, b)
  if a == b then return nil end
  local head, limit = 0, math.min(#a, #b)
  while head < limit and a:byte(head + 1) == b:byte(head + 1) do head = head + 1 end
  while head > 0 and (continuation(a:byte(head + 1)) or continuation(b:byte(head + 1))) do head = head - 1 end
  local tail = 0
  limit = limit - head
  while tail < limit and a:byte(#a - tail) == b:byte(#b - tail) do tail = tail + 1 end
  while tail > 0 and continuation(a:byte(#a - tail + 1)) do tail = tail - 1 end
  return head, #a - head - tail, b:sub(head + 1, #b - tail)
end

local function say(msg)
  vim.fn.chansend(S.job, vim.json.encode(msg) .. "\n")
end

--- Make the buffer show `text`, touching only the lines that differ.
local function show(text)
  local old = api.nvim_buf_get_lines(S.buf, 0, -1, false)
  local new, eol = lines_of(text)
  local head = 0
  while head < #old and head < #new and old[head + 1] == new[head + 1] do head = head + 1 end
  local tail = 0
  while tail < #old - head and tail < #new - head and old[#old - tail] == new[#new - tail] do tail = tail + 1 end
  S.applying = true
  api.nvim_buf_set_lines(S.buf, head, #old - tail, false, vim.list_slice(new, head + 1, #new - tail))
  S.applying = false
  S.shadow, S.eol = text, eol
end

--- The buffer changed under the user's hands: tell the session.
local function typed()
  if S.applying then return end
  local now = text_of(api.nvim_buf_get_lines(S.buf, 0, -1, false), S.eol)
  local at, del, text = diff(S.shadow, now)
  if not at then return end
  S.shadow = now
  S.sent = S.sent + 1
  say({ edit = { at = at, del = del, text = text, seen = S.seen } })
end

local function heard(msg)
  if msg.ready then
    if msg.ready.role == "guest" then
      show(msg.ready.text)
    else
      S.shadow, S.eol = "", true
      typed()
    end
    api.nvim_buf_attach(S.buf, false, {
      on_lines = function() if not S then return true end typed() end,
    })
    S.ready = msg.ready.role
  elseif msg.apply then
    -- Typed since the session wrote this: it no longer fits, and the
    -- session sends what it meant again once it has heard the edit.
    if msg.apply.acked ~= S.sent then return end
    local text = S.shadow
    for _, e in ipairs(msg.apply.edits) do
      text = text:sub(1, e.at) .. e.text .. text:sub(e.at + e.del + 1)
    end
    show(text)
    S.seen = msg.apply.n
    say({ seen = S.seen })
  elseif msg.error then
    api.nvim_echo({ { "v0 live: " .. msg.error, "ErrorMsg" } }, true, {})
    S.error = msg.error
  end
end

function M.start(socket)
  assert(not S, "v0 live is already on in this session")
  local buf = api.nvim_get_current_buf()
  local name = vim.fn.fnamemodify(api.nvim_buf_get_name(buf), ":t")
  local pending = ""
  S = { buf = buf, sent = 0, seen = 0, shadow = "", eol = true, applying = false }
  S.job = vim.fn.jobstart({ vim.g.v0_bin or "v0", "live", socket, name, "--unit", "bytes" }, {
    on_stdout = function(_, data)
      -- Chunks, not lines: the last piece is the start of the next line.
      data[1] = pending .. data[1]
      pending = table.remove(data)
      for _, line in ipairs(data) do heard(vim.json.decode(line)) end
    end,
    on_stderr = function(_, data)
      local err = vim.trim(table.concat(data, "\n"))
      if err ~= "" then api.nvim_echo({ { "v0 live: " .. err, "ErrorMsg" } }, true, {}) end
    end,
    on_exit = function() S = nil end,
  })
end

function M.rename(to)
  local row, col = unpack(api.nvim_win_get_cursor(0))
  S.sent = S.sent + 1
  say({ rename = { at = api.nvim_buf_get_offset(S.buf, row - 1) + col, to = to, seen = S.seen } })
end

function M.hold(on)
  say({ hold = on })
end

function M.state()
  return S
end

api.nvim_create_user_command("V0Live", function(o) M.start(o.args) end, { nargs = 1, complete = "file" })
api.nvim_create_user_command("V0Rename", function(o) M.rename(o.args) end, { nargs = 1 })
api.nvim_create_user_command("V0Hold", function() M.hold(true) end, {})
api.nvim_create_user_command("V0Release", function() M.hold(false) end, {})

return M
