# Why?

- Our company uses Joomla components and PHP, which isn't well supported by **any** PHP LSP
- To learn
- Rust! (blazingly fast)

# Current features

- diagnostics for syntax errors and certain undefined variables (extremely liberal)
- `textDocument/documentSymbol`
- `textDocument/selectionRange`
- code actions
    - convert all `<?php echo ... ?>` calls into `<?= ... ?>` within a file

# Limitations

- no support for file inclusions (`require`, `require_once`, etc.)
- autoload is for aesthetics only and doesn't do anything (yet)
- no autocomplete (most importantly)

# Set up LSP

The following is a crude snippet of my `neovim` configuration.

```lua
local lsp = require('lspconfig')
local lsp_configs = require('lspconfig.configs')
local lsp_util = require('lspconfig.util')

vim.lsp.set_log_level('info')
lsp_configs.my_php_lsp = {
  default_config = {
    cmd = { '/path/to/executable' },
    filetypes = { 'php' },
    root_dir = lsp_util.root_pattern('composer.json', '.git'),

    -- settings
    init_options = {
      diagnostics = {
        syntax = true,
        undefined = false,
      },
    },

    capabilities = vim.lsp.protocol.make_client_capabilities(),
  }
}

lsp.my_php_lsp.setup({})
```

# Dev

```console
$ git submodule init
$ git submodule update
$ cargo test
```
