use serde::Serialize;

#[derive(Debug, Clone, Serialize)]
pub struct ConnectionStates {
    pub established: u32,
    pub time_wait: u32,
    pub close_wait: u32,
    pub fin_wait_1: u32,
    pub fin_wait_2: u32,
    pub syn_sent: u32,
    pub syn_received: u32,
    pub closing: u32,
    pub last_ack: u32,
    pub listen: u32,
}

pub async fn collect() -> Option<ConnectionStates> {
    #[cfg(windows)]
    let output = tokio::process::Command::new("netstat")
        .args(["-an"])
        .output()
        .await;

    #[cfg(unix)]
    let output = tokio::process::Command::new("netstat")
        .args(["-an"])
        .output()
        .await;

    let output = output.ok()?;
    let text = String::from_utf8_lossy(&output.stdout);

    let mut states = ConnectionStates {
        established: 0,
        time_wait: 0,
        close_wait: 0,
        fin_wait_1: 0,
        fin_wait_2: 0,
        syn_sent: 0,
        syn_received: 0,
        closing: 0,
        last_ack: 0,
        listen: 0,
    };

    for line in text.lines() {
        let line = line.trim().to_uppercase();
        if line.contains("ESTABLISHED") {
            states.established += 1;
        } else if line.contains("TIME_WAIT") {
            states.time_wait += 1;
        } else if line.contains("CLOSE_WAIT") {
            states.close_wait += 1;
        } else if line.contains("FIN_WAIT_1") || line.contains("FIN_WAIT1") {
            states.fin_wait_1 += 1;
        } else if line.contains("FIN_WAIT_2") || line.contains("FIN_WAIT2") {
            states.fin_wait_2 += 1;
        } else if line.contains("SYN_SENT") {
            states.syn_sent += 1;
        } else if line.contains("SYN_RECV") || line.contains("SYN_RECEIVED") {
            states.syn_received += 1;
        } else if line.contains("CLOSING") {
            states.closing += 1;
        } else if line.contains("LAST_ACK") {
            states.last_ack += 1;
        } else if line.contains("LISTEN") || line.contains("LISTENING") {
            states.listen += 1;
        }
    }

    Some(states)
}
