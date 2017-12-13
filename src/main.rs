extern crate termion;
extern crate tui;
extern crate rsmq;
extern crate chrono;
extern crate clap;

use std::io;
use std::thread;
use std::time;
use std::sync::mpsc;

use termion::event;
use termion::input::TermRead;

use tui::Terminal;
use tui::backend::MouseBackend;
use tui::widgets::{border, Block, Table, SelectableList, Widget, Row};
use tui::layout::{Direction, Group, Rect, Size};
use tui::style::{Color, Modifier, Style};

use chrono::NaiveDateTime;
use clap::{Arg};

struct App {
    rsmq: rsmq::Rsmq,
    size: Rect,
    queue_names: Vec<String>,
    selected: usize,
    prev_selected: usize,
    selected_q: Option<rsmq::Queue>,
}

impl App {
    fn new(redis_url: &str, redis_ns: &str) -> App {
        let rsmq = rsmq::Rsmq::new(redis_url, redis_ns).expect("Can't instantiate RSMQ");
        let (qnames, q) = Self::fetch_queue_data(&rsmq, 0);

        App {
            rsmq: rsmq,
            size: Rect::default(),
            queue_names: qnames, 
            selected: 0,
            prev_selected: 0,
            selected_q: q,
        }
    }
    fn advance(&mut self) {
        // TODO: selection is messed up by the sorting – need to keep track of which item is selected and make sure it is still selected after refresh the new queues. E.g. if item 0 is selected and I add a new queue that replaces the previous queue at index 0, then the selection changes
        if self.selected != self.prev_selected {
            let (qnames, q) = Self::fetch_queue_data(&self.rsmq, self.selected);
            self.prev_selected = self.selected;
            self.queue_names = qnames;
            self.selected_q = q;
        }
    }

    fn fetch_queue_data(rsmq: &rsmq::Rsmq, idx: usize) -> (Vec<String>, Option<rsmq::Queue>) {
        let mut queue_names = rsmq.list_queues().expect("Can't fetch queue list");

        let selected_q = if queue_names.is_empty() || idx > queue_names.len() {
            None
        } else {
            queue_names.sort();
            let q = rsmq.get_queue_attributes(&queue_names[idx]).expect("Can't fetch queue attributes");
            Some(q)
        };
        (queue_names, selected_q)
    }
}

enum Event {
    Input(event::Key),
    Tick,
}


fn main() {
    // Parse command line args
    let cli_args = clap::App::new("RSMQ dashboard")
        .version("0.1")
        .about("Terminal UI for your RSMQ queues")
        .arg(Arg::with_name("namespace")
            .short("n")
            .long("namespace")
            .value_name("REDIS_NS")
            .help("key used to namespace the RSMQ data in Redis")
        )
        .arg(Arg::with_name("redis-url")
            .short("r")
            .long("redis-url")
            .value_name("REDIS_URL")
            .help("Redis connection string on the format redis://HOST[:PORT][?password=PASSWORD[&db=DATABASE]]")
        )
        .get_matches();
    let redis_ns = cli_args.value_of("namespace").unwrap_or("rsmq");
    let redis_url = cli_args.value_of("redis-url").unwrap_or("redis://127.0.0.1:8909");

    // Terminal initialization
    let backend = MouseBackend::new().unwrap();
    let mut terminal = Terminal::new(backend).unwrap();

    // Channels
    let (tx, rx) = mpsc::channel();
    let input_tx = tx.clone();
    let clock_tx = tx.clone();

    // Input
    thread::spawn(move || {
        let stdin = io::stdin();
        for c in stdin.keys() {
            let evt = c.unwrap();
            input_tx.send(Event::Input(evt)).unwrap();
            if evt == event::Key::Char('q') {
                break;
            }
        }
    });

    // Tick
    thread::spawn(move || loop {
        clock_tx.send(Event::Tick).unwrap();
        thread::sleep(time::Duration::from_millis(1000));
    });

    // App
    let mut app = App::new(redis_url, redis_ns);

    // First draw call
    terminal.clear().unwrap();
    terminal.hide_cursor().unwrap();
    app.size = terminal.size().unwrap();
    draw(&mut terminal, &app);

    loop {
        let size = terminal.size().unwrap();
        if size != app.size {
            terminal.resize(size).unwrap();
            app.size = size;
        }

        let evt = rx.recv().unwrap();
        match evt {
            Event::Input(input) => match input {
                event::Key::Char('q') => {
                    terminal.clear().expect("Could not clear terminal");
                    terminal.show_cursor().expect("Could not show cursor");
                    break;
                }
                event::Key::Down => {
                    app.selected += 1;
                    if app.selected > app.queue_names.len() - 1 {
                        app.selected = 0;
                    }
                    app.advance();
                }
                event::Key::Up => {
                    if app.selected > 0 {
                        app.selected -= 1;
                    } else {
                        app.selected = app.queue_names.len() - 1;
                    }
                    app.advance();                    
                },
                _ => {}
            },
            Event::Tick => {
                app.advance();
            }
        }
        draw(&mut terminal, &app);
    } // end loop

    terminal.show_cursor().unwrap();
}

fn draw(t: &mut Terminal<MouseBackend>, app: &App) {
    Group::default()
        .direction(Direction::Horizontal)
        .sizes(&[Size::Percent(20), Size::Percent(80)])
        .render(t, &app.size, |t, chunks| {
            SelectableList::default()
                .block(Block::default().borders(border::ALL).title(" Queues "))
                .items(&app.queue_names)
                .select(app.selected)
                .highlight_style(Style::default().fg(Color::Yellow).modifier(Modifier::Bold))
                .highlight_symbol(">")
                .render(t, &chunks[0]);

            if let &Some(ref q) = &app.selected_q {
                let created_at = NaiveDateTime::from_timestamp_opt(q.created as i64, 0).unwrap_or(NaiveDateTime::from_timestamp(0,0)).format("%Y-%m-%d %H:%M:%S");
                let modified_at = NaiveDateTime::from_timestamp_opt(q.modified as i64, 0).unwrap_or(NaiveDateTime::from_timestamp(0,0)).format("%Y-%m-%d %H:%M:%S");
                let items : Vec<Vec<String>> = vec![
                    vec!["Name".to_string(), q.qname.to_string()],
                    vec!["Reservation time".to_string(), q.vt.to_string()],
                    vec!["Initial delay".to_string(), q.delay.to_string()],
                    vec!["Max msg size".to_string(), q.maxsize.to_string()],
                    vec!["Total msg sent".to_string(), q.totalsent.to_string()],
                    vec!["Total msg received".to_string(), q.totalrecv.to_string()],
                    vec!["Created at".to_string(), created_at.to_string()],
                    vec!["Modified at".to_string(), modified_at.to_string()],
                ];
                let normal_style = Style::default().fg(Color::White);
                Table::new(
                    ["",""].into_iter(),
                    items.iter().map(|queue_row| Row::StyledData(queue_row.into_iter(), &normal_style)),
                ).block(Block::default().borders(border::ALL).title(&format!(" Details: {} ",&q.qname)))
                    .widths(&[18, 19])
                    .render(t, &chunks[1]);
            }
        });

    t.draw().unwrap();
}
