-- Every run starts with this: the `pc` namespace, `print` into the run's
-- output, `show` and `help`.

local call = function(id, args) return __pc_call(id, args) end

local function namespace(path)
  return setmetatable({}, {
    __index = function(_, key)
      return namespace(path == "" and key or path .. "." .. key)
    end,
    __call = function(_, args) return call(path, args) end,
    __tostring = function() return "commands " .. path end,
  })
end
pc = namespace("")

function print(...)
  local parts = {}
  for i = 1, select("#", ...) do
    parts[#parts + 1] = tostring((select(i, ...)))
  end
  __pc_print(table.concat(parts, "\t"))
end

function show(value, indent)
  indent = indent or ""
  if type(value) == "string" then return string.format("%q", value) end
  if type(value) ~= "table" then return tostring(value) end
  local keys = {}
  for k in pairs(value) do keys[#keys + 1] = k end
  if #keys == 0 then return "{}" end
  table.sort(keys, function(a, b)
    if type(a) == type(b) then return a < b end
    return type(a) == "number"
  end)
  local inner = indent .. "  "
  local lines = {}
  for _, k in ipairs(keys) do
    local key = type(k) == "number" and "" or (k .. " = ")
    lines[#lines + 1] = inner .. key .. show(value[k], inner)
  end
  return "{\n" .. table.concat(lines, ",\n") .. "\n" .. indent .. "}"
end

function help(prefix)
  for _, c in ipairs(call("app.commands", { prefix = prefix })) do
    local names = {}
    for _, p in ipairs(c.params) do
      names[#names + 1] = p.required and p.name or (p.name .. "?")
    end
    if c.extra_args then names[#names + 1] = "..." end
    print(string.format("pc.%s{%s}  %s", c.id, table.concat(names, ", "), c.summary))
  end
end
