use std::io::{self};
use std::net::SocketAddr;
use std::sync::mpsc::{self, Receiver, Sender};
use std::time::{Duration, Instant};

use clap::Parser;
use crossterm::event::{self, DisableMouseCapture, EnableMouseCapture, Event as CEvent, KeyCode, KeyEventKind};
use crossterm::terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen};
use ratatui::backend::CrosstermBackend;
use ratatui::layout::{Constraint, Direction, Layout};
use ratatui::text::{Span, Line};
use ratatui::widgets::{Block, Borders, Paragraph, Wrap};
use ratatui::Terminal;
use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
use tokio::net::{TcpListener, TcpStream};

#[derive(Parser, Debug)]
#[command(author, version, about = "1:1 chat TUI using ratatui + sockets", long_about = None)]
struct Args {
    /// mode: server or client
    #[arg(short, long, default_value = "server")]
    mode: String,

    /// address to bind/connect, like 127.0.0.1:9000
    #[arg(short, long, default_value = "127.0.0.1:9000")]
    addr: String,

    /// your display name
    #[arg(short, long, default_value = "you")]
    name: String,
}

enum NetworkCommand {
    Send(String),
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let args = Args::parse();
    let addr: SocketAddr = args.addr.parse()?;

    let (net_tx, net_rx): (Sender<NetworkCommand>, Receiver<NetworkCommand>) = mpsc::channel();
    let (ui_tx, ui_rx): (Sender<String>, Receiver<String>) = mpsc::channel();

    let name_clone = args.name.clone();
    let mode = args.mode.clone();
    tokio::spawn(async move {
        if mode == "server" {
            if let Err(e) = run_server(addr, name_clone, net_rx, ui_tx).await {
                eprintln!("server error: {e}");
            }
        } else {
            if let Err(e) = run_client(addr, name_clone, net_rx, ui_tx).await {
                eprintln!("client error: {e}");
            }
        }
    });

    run_ui(ui_rx, net_tx, args.name)
}

async fn run_server(
    addr: SocketAddr,
    name: String,
    net_rx: Receiver<NetworkCommand>,
    ui_tx: Sender<String>,
) -> anyhow::Result<()> {
    println!("Starting server on {addr} - waiting for one connection...");
    let listener = TcpListener::bind(addr).await?;
    let (socket, peer) = listener.accept().await?;
    println!("Client connected: {peer}");
    ui_tx.send(format!("--- Connected: {peer} ---")).ok();

    handle_socket(socket, name, net_rx, ui_tx).await
}

async fn run_client(
    addr: SocketAddr,
    name: String,
    net_rx: Receiver<NetworkCommand>,
    ui_tx: Sender<String>,
) -> anyhow::Result<()> {
    println!("Connecting to {addr}...");
    let socket = TcpStream::connect(addr).await?;
    println!("Connected to server");
    ui_tx.send("--- Connected to server ---".into()).ok();

    handle_socket(socket, name, net_rx, ui_tx).await
}

async fn handle_socket(
    socket: TcpStream,
    name: String,
    net_rx: Receiver<NetworkCommand>,
    ui_tx: Sender<String>,
) -> anyhow::Result<()> {
    let (reader, mut writer) = socket.into_split();
    let mut lines = BufReader::new(reader).lines();

    let ui_tx_clone = ui_tx.clone();

    loop {
        tokio::select! {
            maybe = lines.next_line() => {
                match maybe {
                    Ok(Some(line)) => {
                        ui_tx_clone.send(line).ok();
                    }
                    Ok(None) => {
                        ui_tx_clone.send("--- Connection closed by peer ---".into()).ok();
                        break;
                    }
                    Err(e) => {
                        ui_tx_clone.send(format!("--- Socket read error: {e} ---")).ok();
                        break;
                    }
                }
            }

            _ = tokio::time::sleep(Duration::from_millis(50)) => {
                while let Ok(cmd) = net_rx.try_recv() {
                    match cmd {
                        NetworkCommand::Send(msg) => {
                            let out = format!("{}: {}\n", name, msg);
                            if let Err(e) = writer.write_all(out.as_bytes()).await {
                                ui_tx_clone.send(format!("--- Socket write error: {e} ---")).ok();
                                return Ok(());
                            }
                        }
                    }
                }
            }
        }
    }

    Ok(())
}


fn run_ui(ui_rx: Receiver<String>, net_tx: Sender<NetworkCommand>, my_name: String) -> anyhow::Result<()> {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    crossterm::execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(io::stdout());
    let mut terminal = Terminal::new(backend)?;

    let res = ui_loop(&mut terminal, ui_rx, net_tx, my_name);

    disable_raw_mode()?;
    crossterm::execute!(io::stdout(), LeaveAlternateScreen, DisableMouseCapture)?;
    terminal.show_cursor()?;

    res
}

fn ui_loop(
    terminal: &mut Terminal<CrosstermBackend<io::Stdout>>,
    ui_rx: Receiver<String>,
    net_tx: Sender<NetworkCommand>,
    my_name: String,
) -> anyhow::Result<()> {
    let mut messages: Vec<String> = Vec::new();
    let mut input = String::new();
    let tick_rate = Duration::from_millis(100);
    let mut last_tick = Instant::now();

    loop {
        while let Ok(line) = ui_rx.try_recv() {
            messages.push(line);
        }

        terminal.draw(|f| {
            let size = f.area();
            let chunks = Layout::default()
                .direction(Direction::Vertical)
                .constraints([Constraint::Min(1), Constraint::Length(3)].as_ref())
                .split(size);

            // messages를 Line 벡터로 변환
            let text: Vec<Line> = messages
                .iter()
                .rev()
                .take(1000)
                .rev()
                .map(|m| Line::from(Span::raw(m.clone())))
                .collect();

            let messages_block = Paragraph::new(text)
                .block(Block::default().borders(Borders::ALL).title("Messages"))
                .wrap(Wrap { trim: false });

            // input도 Line으로 감싸기
            let input_block = Paragraph::new(vec![
                Line::from(Span::raw(input.as_str()))
            ])
                .block(Block::default().borders(Borders::ALL).title(format!("Type and press Enter ({})", my_name)));

            f.render_widget(messages_block, chunks[0]);
            f.render_widget(input_block, chunks[1]);
        })?;

        let timeout = tick_rate.checked_sub(last_tick.elapsed()).unwrap_or_else(|| Duration::from_secs(0));
        if event::poll(timeout)? {
            if let CEvent::Key(key_event) = event::read()? {
                if key_event.kind == KeyEventKind::Press {
                    match key_event.code {
                        KeyCode::Char(c) => {
                            input.push(c);
                        }
                        KeyCode::Backspace => { input.pop(); }
                        KeyCode::Enter => {
                            if !input.trim().is_empty() {
                                let to_send = input.clone();
                                messages.push(format!("{}: {}", my_name, to_send));
                                net_tx.send(NetworkCommand::Send(to_send)).ok();
                                input.clear();
                            }
                        }
                        KeyCode::Esc => {
                            messages.push("--- Exiting ---".into());
                            break;
                        }
                        _ => {}
                    }
                }
            }
        }

        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }

    Ok(())
}
