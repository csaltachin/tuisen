# tuisen

This is a rudimentary Twitch chat client for the terminal, built using the [Ratatui](https://github.com/ratatui-org/ratatui) library. It is currently a work in progress and mainly a Rust learning project; if you're looking for a more mature and featureful TUI client, check out [twitch-tui](https://github.com/Xithrius/twitch-tui) instead. (Heavy inspiration is drawn from there.)

If you do want to try tuisen out, then:

1. Make sure you have Rust/`cargo` installed. You can find instructions [here](https://www.rust-lang.org/tools/install).
2. Clone this repository, and `cd` into the repo's root directory.
3. Create a configuration file named `tuisen.toml`. Write your username and OAuth token, and a channel to join, following the example in `tuisen_example.toml`. You can get a token for your Twitch account [here](https://twitchapps.com/tmi/), or [here](https://twitchtokengenerator.com/) if you want to customize your scopes (although tuisen only really needs `chat:read` and `chat:edit` for now). See the Twitch API documentation for more about [authenticating with OAuth tokens](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/) and [chat scopes](https://dev.twitch.tv/docs/authentication/scopes/#chat-and-pubsub-scopes).
4. Run the client with `cargo run --release`. If you don't specify a username/token pair in the config file, the default behavior is to connect anonymously -- you will be able to receive chat messages but not send them. If you don't specify a channel to join, the client will connect to a hard-coded default channel, which is currently `forsen`. It is a planned feature to allow users to specify a channel after they start the client.
5. Exit the client by pressing `<Esc>` at any time.

Be mindful about storing tokens as plaintext; this is potentially unsafe. Both convenience methods linked above have instructions for revoking OAuth tokens. If you obtained a token using your own `client_id`, you can find instructions for revoking it [here](https://dev.twitch.tv/docs/authentication/revoke-tokens/). If you fork this repo, make sure to keep `tuisen.toml` in your `.gitignore` (as it is here).
