### tuisen

This is a rudimentary Twitch chat client for the terminal, built using the [Ratatui](https://github.com/ratatui-org/ratatui) library. It is currently a work in progress and mainly a Rust learning project; if you're looking for a more mature and featureful TUI client, check out [twitch-tui](https://github.com/Xithrius/twitch-tui) instead.

If you do want to try it out, then:

1. Make sure you have Rust/`cargo` installed (instructions [here](https://www.rust-lang.org/tools/install)).
2. Clone this repository, and `cd` into the repo's root directory.
3. Create a configuration file named `tuisen.toml`, and write your username and OAuth token following the format in `tuisen_example.toml`. You can get a token for your Twitch account [here](https://twitchapps.com/tmi/), or [here](https://twitchtokengenerator.com/) if you want to customize your scopes (although tuisen only really needs `chat:read` and `chat:edit` for now). See the Twitch API documentation for more about [authenticating with OAuth tokens](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/) and [chat scopes](https://dev.twitch.tv/docs/authentication/scopes/#chat-and-pubsub-scopes). Be careful -- storing tokens as plaintext is potentially unsafe.
4. Run the client with `cargo run --release`. If you didn't specify your username/token in `tuisen.toml`, the default behavior is to connect anonymously -- you will be able to receive chat messages but not send them. The channel to join is currently hardcoded in `main.rs` as `DEFAULT_CHANNEL`, but soon you'll be able to specify it in the config file, and I plan to make it so that you can also connect after starting the client.
5. Exit the client by pressing `<Esc>` at any time.
