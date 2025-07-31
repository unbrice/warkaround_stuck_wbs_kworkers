# Nix and NixOS

Using Nix and NixOS to develop, build, and run.

## Building

```sh
git clone https://github.com/unbrice/stuck_writeback_workaround.git
cd stuck_writeback_workaround
nix run
```

## Development

For development or to build the binary from source, you can use the provided Nix flake to enter a development shell with all the necessary dependencies.

```sh
git clone https://github.com/unbrice/stuck_writeback_workaround.git
cd stuck_writeback_workaround
nix develop
# ... edits ....
cargo run -- --help
```


## NixOS Installation

For NixOS users, a flake is available to install and run the workaround as a systemd service.

1.  **Add the flake to your `flake.nix` inputs:**

    ```nix
    inputs.stuck_writeback_workaround.url = "github:unbrice/stuck_writeback_workaround";
    ```

2.  **Add the module to your NixOS configuration:**

    ```nix
    # In your configuration.nix or a file imported by it
    {
      imports = [
        inputs.stuck_writeback_workaround.nixosModules.default
      ];

      services.stuck-writeback-workaround.enable = true;
    }
    ```

3.  **Rebuild your NixOS system:**

    ```sh
    sudo nixos-rebuild switch --flake
    ```

### NixOS Module Options

- `services.stuck-writeback-workaround.processGlob`: Glob pattern for matching kworker process names.
  - **Type**: String
  - **Default**: `"kworker/*inode_switch_wbs*"`

- `services.stuck-writeback-workaround.runtimeThreshold`: The maximum time a kworker process can run before a `sync` is triggered.
  - **Type**: String (e.g., `"30s"`, `"1m"`)
  - **Default**: `"30s"` 