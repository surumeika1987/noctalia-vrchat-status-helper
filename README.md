# VRChat Status Helper

A helper application that exchanges data with the Noctalia plugin `VRChat Status`.  
It communicates with the **unofficial** VRChat API using the `vrchatapi` crate.

## Disclaimer

The VRChat API is **unofficial**.  
The developer is not responsible for any issues arising from use of this software.  
Use this software at your own risk.

## Installation

A Rust build environment is required.  
Run the following commands to build and install the helper:

```sh
git clone https://github.com/surumeika1987/noctalia-vrchat-status-helper.git
cd noctalia-vrchat-status-helper
cargo build --release
mkdir -p ~/.local/bin
cp ./target/release/vrchat-status-helper ~/.local/bin/
```

## Usage

Use this helper together with the Noctalia plugin `VRChat Status`.  
You can install the plugin manually with the following command:

```sh
git clone https://github.com/surumeika1987/noctalia-vrchat-status.git \
    ~/.local/share/noctalia/plugins/vrchat-status
```

### Logging In

Log in to VRChat with the following command.  
You need to log in on first launch and whenever your authentication credentials expire.

```sh
vrchat-status-helper login
```

Authentication credentials are stored in
`$XDG_CACHE_HOME/noctalia/vrchat-status/cookies.txt`, or in
`~/.cache/noctalia/vrchat-status/cookies.txt`, with `0600` permissions.  
Handle your authentication credentials with care.

### Starting the Helper

When started without arguments, `vrchat-status-helper` runs as a background daemon.  
We recommend configuring your window manager, such as Hyprland, to start it automatically.

```lua
hl.on("hyprland.start", function()
    hl.exec_cmd("noctalia")
    hl.exec_cmd("/home/<your name>/.local/bin/vrchat-status-helper")
end)
```

### VRChat API Access

To reduce load on the VRChat API, this software limits API access to once every 60 seconds.  
As a result, changes may take some time to appear in VRChat.

### For Developers

This software accepts IPC messages over a Unix domain socket.  
Use the following command to communicate with it over IPC:

```sh
vrchat-status-helper msg <payload>
```

The payload format is `<status-number>:<status-message>`.  
The status message may be empty.  
The status numbers are as follows:

| Number | VRChat Status |
| --- | --- |
| 4 | Join Me |
| 3 | Online |
| 2 | Ask Me |
| 1 | Do Not Disturb |
| 0 | Offline |

Set `RUST_LOG=debug` when starting the helper to enable debug logging:

```sh
RUST_LOG=debug vrchat-status-helper
```
