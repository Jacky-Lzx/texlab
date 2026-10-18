-- TEXLAB_BINARY=/path/to/texlab nvim --clean --headless -i NONE -l tests/build_log/run.lua
local suite = vim.fn.fnamemodify(arg[0], ":p:h")
local binary = assert(vim.env.TEXLAB_BINARY, "Set TEXLAB_BINARY to the texlab build under test")
assert(vim.fn.executable(binary) == 1, "texlab binary is not executable")
assert(vim.fn.executable("latexmk") == 1, "latexmk and pdfLaTeX must be installed")
local directory = vim.env.TEXLAB_TEST_OUTPUT
	or (assert(vim.uv.os_tmpdir()) .. "/texlab-build-log-" .. vim.fn.getpid() .. "-" .. os.time())
assert(
	vim.fn.isdirectory(directory) == 0 or #vim.fn.readdir(directory) == 0,
	"Choose an empty TEXLAB_TEST_OUTPUT directory"
)
vim.fn.mkdir(directory, "p")
vim.o.swapfile, vim.o.shadafile = false, "NONE"
local cases = vim.json.decode(
	table.concat(vim.fn.readfile(suite .. "/cases.json"), "\n"),
	{ luanil = { object = true, array = true } }
)
local reports, failures, known = {}, 0, 0
local function write(path, text)
	vim.fn.mkdir(vim.fn.fnamemodify(path, ":h"), "p")
	local file = assert(io.open(path, "wb"))
	file:write(text)
	file:close()
end
for _, case in ipairs(cases) do
	if not vim.env.TEXLAB_TEST_FILTER or case.name:find(vim.env.TEXLAB_TEST_FILTER, 1, true) then
		local dir = directory .. "/" .. case.name
		for path, text in pairs(case.files) do
			write(dir .. "/" .. path, text)
		end
		write(dir .. "/.texlabroot", "")
		local root = case.root or "main.tex"
		local cmd = { "latexmk", "-norc", "-pdf", "-interaction=nonstopmode", "-recorder" }
		if case.explicit then
			cmd[#cmd + 1] = "-file-line-error"
		end
		vim.list_extend(cmd, case.compiler_args or {})
		cmd[#cmd + 1] = root
		local build = vim.system(cmd, { cwd = dir, env = case.env, text = true }):wait(30000)
		write(dir .. "/compiler.txt", (build.stdout or "") .. (build.stderr or ""))
		local raw, events, client = {}, 0, nil
		local report = { name = case.name, build_code = build.code, failures = {} }
		local function check(condition, message)
			if not condition then
				report.failures[#report.failures + 1] = message
			end
		end
		local build_valid = case.success and build.code == 0 or not case.success and build.code == 12
		local log_name = vim.fn.fnamemodify(root, ":t:r") .. ".log"
		local log_exists = vim.fn.filereadable(dir .. "/" .. log_name) == 1
			or vim.fn.filereadable(dir .. "/" .. vim.fn.fnamemodify(root, ":h") .. "/" .. log_name) == 1
		check(build_valid, "Unexpected compiler exit code: " .. build.code)
		check(log_exists, "Compiler did not produce a log")
		local ok, err = xpcall(function()
			vim.cmd.edit(vim.fn.fnameescape(dir .. "/" .. root))
			local id = assert(vim.lsp.start({
				name = "texlab-corner",
				cmd = { binary },
				root_dir = dir,
				settings = { texlab = { diagnosticsDelay = 20, chktex = { onOpenAndSave = false, onEdit = false } } },
				handlers = {
					["textDocument/publishDiagnostics"] = function(_, result)
						if result then
							raw[result.uri] = result.diagnostics
							events = events + 1
						end
					end,
				},
			}))
			client = assert(vim.lsp.get_client_by_id(id))
			assert(
				vim.wait(10000, function()
					return client.initialized
				end, 10),
				"LSP initialization timed out"
			)
			-- Open every fixture source, including extensions not inferred as TeX.
			for path, text in pairs(case.files) do
				if path ~= root then
					client:notify("textDocument/didOpen", {
						textDocument = {
							uri = vim.uri_from_fname(dir .. "/" .. path),
							languageId = "latex",
							version = 1,
							text = text,
						},
					})
				end
			end
			local previous, stable = -1, 0
			assert(
				vim.wait(5000, function()
					if events > 0 and events == previous then
						stable = stable + 1
					else
						stable = 0
					end
					previous = events
					return stable >= 50
				end, 20),
				"Diagnostics did not settle"
			)
			local actual = {}
			for uri, ds in pairs(raw) do
				for _, d in ipairs(ds) do
					if d.source == "latex" then
						actual[#actual + 1] = {
							file = vim.uri_to_fname(uri):sub(#dir + 2),
							range = d.range,
							message = d.message,
							severity = d.severity,
						}
					end
				end
			end
			table.sort(actual, function(a, b)
				if a.file ~= b.file then
					return a.file < b.file
				end
				if a.range.start.line ~= b.range.start.line then
					return a.range.start.line < b.range.start.line
				end
				if a.range.start.character ~= b.range.start.character then
					return a.range.start.character < b.range.start.character
				end
				return a.message < b.message
			end)
			report.actual = actual
			check(
				#actual == #case.expected,
				("Expected %d compiler diagnostics, got %d"):format(#case.expected, #actual)
			)
			local used = {}
			for _, expected in ipairs(case.expected) do
				local found
				for i, d in ipairs(actual) do
					if
						not used[i]
						and d.message:find(expected.message, 1, true)
						and (not expected.file or expected.file == d.file)
						and (not expected.line or expected.line - 1 == d.range.start.line)
					then
						found = i
						break
					end
				end
				check(found, "Missing expected diagnostic: " .. vim.json.encode(expected))
				if found then
					used[found] = true
					local d = actual[found]
					check(d.severity == (expected.severity or 1), "Wrong diagnostic severity")
					if expected.line and expected.severity ~= 2 then
						local col, end_col = 0, 0
						if expected.token then
							local lines = vim.split(case.files[expected.file], "\n", { plain = true })
							local line = lines[expected.line]:gsub("\r$", "")
							local pos, start = 0, 1
							for _ = 1, expected.occurrence or 1 do
								pos = assert(line:find(expected.token, start, true))
								start = pos + #expected.token
							end
							col = vim.str_utfindex(line:sub(1, pos - 1), "utf-16")
							end_col = col + vim.str_utfindex(expected.token, "utf-16")
						elseif expected.message ~= "Undefined control sequence." then
							-- Other TeX errors retain context ranges; assert their reported line.
							col, end_col = d.range.start.character, d.range["end"].character
						end
						check(
							d.range.start.character == col
								and d.range["end"].character == end_col
								and d.range["end"].line == expected.line - 1,
							"Wrong range: " .. vim.json.encode(d)
						)
					end
				end
			end
		end, debug.traceback)
		if not ok then
			report.failures[#report.failures + 1] = err
		end
		if client then
			client:stop(true)
			vim.wait(3000, function()
				return client:is_stopped()
			end, 10)
		end
		local status = #report.failures == 0 and "PASS" or "FAIL"
		if case.known_limitation and ok and build_valid and log_exists then
			if #report.failures > 0 then
				status = "XFAIL"
				known = known + 1
				report.known_limitation = case.known_limitation
			else
				status = "XPASS"
				report.failures = { "Known limitation no longer reproduced; remove annotation" }
			end
		end
		report.status = status
		if status == "FAIL" or status == "XPASS" then
			failures = failures + 1
		end
		reports[#reports + 1] = report
		print(
			status .. " " .. case.name .. (#report.failures > 0 and ": " .. table.concat(report.failures, "; ") or "")
		)
		write(directory .. "/results.json", vim.json.encode(reports) .. "\n")
	end
end
assert(#reports > 0, "No cases selected")
print(
	("%d passed, %d known limitations, %d failed (%d cases); artifacts: %s"):format(
		#reports - failures - known,
		known,
		failures,
		#reports,
		directory
	)
)
if failures > 0 then
	vim.cmd("cquit 1")
else
	vim.cmd("qa!")
end
