# tuisen

This is a rudimentary Twitch chat client for the terminal, built using the [Ratatui](https://github.com/ratatui-org/ratatui) library. It is currently a work in progress and mainly a Rust learning project; if you're looking for a more mature and featureful TUI client, check out [twitch-tui](https://github.com/Xithrius/twitch-tui) instead. (Heavy inspiration is drawn from there.)

If you do want to try tuisen out, then:

1. Make sure you have `cargo` installed. You can find instructions [here](https://www.rust-lang.org/tools/install).
2. Clone this repository, and `cd` into the repo's root directory.
3. Create a configuration file named `tuisen.toml`. Write your username, OAuth token (without the `"oauth:"` prefix) and a channel to join, following the example in `tuisen_example.toml`. See more about tokens below.
4. Compile and run the client with `cargo run --release`. If you don't specify a channel to join, the client will connect to a hard-coded default channel, which is currently `forsen`. It is a planned feature to allow users to specify a channel after they start the client.
5. To exit the client, press `<Ctrl-q>` at any time, or press `<q>` in normal mode. See below about modes.

## Modes

tuisen has two modes: *normal mode* and *insert mode*. When you open the client, you start out in normal mode with these keybinds:

* `<q>` exits the client.
* `<i>` enters insert mode.
* `<Up>` and `<Down>` scroll the chat up and down by one line; `<Home>` and `<End>` scroll to the top and bottom of the chat, respectively. These only work when the chat lines overflow the chat window.

When you enter insert mode, the input box is highlighted and the cursor is shown. You can type a message and press `<Enter>` to send it to the current channel. To go back to normal mode, press `<Esc>`.

As mentioned earlier, you can force-quit tuisen at any time by pressing `<Ctrl-q>` in any mode.

## Tokens

You need an OAuth token for your Twitch account in order to use it with tuisen. If you don't specify a username/token pair in the config file, the default behavior is to connect anonymously -- you will be able to receive chat messages but not send them.

Usually, to get a token, you need to obtain your own `client_id`, which requires you to register an application in your Twitch developer console. To avoid this hassle, here are two convenient resources to just get a working token:

* [twitchapps.com/tmi](https://twitchapps.com/tmi/).
* [twitchtokengenerator.com](https://twitchtokengenerator.com/). This one allows you to use your own `client_id` and `client_secret`, and to customize your *chat scopes* if you wish to. Currently, tuisen only really needs `chat:read` and `chat:edit`.

Note that I'm not affiliated with either of these websites -- my understanding is that they use their own `client_id` to obtain tokens, and they use an authorization flow that does not store your token on their end. Ideally, in the future, tuisen would have its own version of this that integrates with the terminal app, like [Chatterino](https://github.com/Chatterino/chatterino2) does. See the Twitch API documentation for more details about [authenticating with OAuth tokens](https://dev.twitch.tv/docs/authentication/getting-tokens-oauth/) and [chat scopes](https://dev.twitch.tv/docs/authentication/scopes/#chat-and-pubsub-scopes).

Be mindful about storing tokens as plaintext; this is potentially unsafe. Both of the above resources include instructions for revoking OAuth tokens. If you obtained a token using your own `client_id`, you can find instructions for revoking it [here](https://dev.twitch.tv/docs/authentication/revoke-tokens/). 

Finally, if you fork this repo, make sure to keep `tuisen.toml` in your `.gitignore` (as it is here) so you don't leak your own token!
