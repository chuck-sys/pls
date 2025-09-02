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
vim.lsp.config('my_pls', {
  cmd = {
    '/path/to/executable',
    '/path/to/executable/phpstorm-stubs/PhpStormStubsMap.php',
  },
  filetypes = { 'php' },
  root_markers = { 'composer.json', '.git' },

  init_options = {
    diagnostics = {
      syntax = true,
      undefined = true,
    },
  },
})

vim.lsp.enable('my_pls')
```

# Dev

```console
$ git submodule init
$ git submodule update
$ cargo test
```
