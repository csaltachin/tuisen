pub enum TwitchAction {
    Connect,
    SendPrivmsg { message: String },
}

pub enum TerminalAction {
    PrintPrivmsg {
        channel: String,
        username: String,
        message: String,
    },
    PrintPing(String),
    PrintDebug(String),
}
