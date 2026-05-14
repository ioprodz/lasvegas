use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

const INDEX_HTML: &str = include_str!("../../static/index.html");
const STYLE_CSS: &str = include_str!("../../static/style.css");
const CSS_TOKENS: &str = include_str!("../../static/css/tokens.css");
const CSS_LAYOUT: &str = include_str!("../../static/css/layout.css");
const CSS_COMPONENTS: &str = include_str!("../../static/css/components.css");
const CSS_PAGES: &str = include_str!("../../static/css/pages.css");
const JS_WEBSOCKET: &str = include_str!("../../static/js/websocket.js");
const JS_LED_PREVIEW: &str = include_str!("../../static/js/led-preview.js");
const JS_TOPBAR: &str = include_str!("../../static/js/topbar.js");
const JS_NAVIGATION: &str = include_str!("../../static/js/navigation.js");
const JS_ANIMATION: &str = include_str!("../../static/js/animation.js");
const JS_AUDIO_ANALYSIS: &str = include_str!("../../static/js/audio-analysis.js");
const JS_VISUALIZER: &str = include_str!("../../static/js/visualizer.js");
const JS_CHROMATIC_CIRCLE: &str = include_str!("../../static/js/chromatic-circle.js");
const JS_AUDIO_CAPTURE: &str = include_str!("../../static/js/audio-capture.js");
const JS_CALIBRATION: &str = include_str!("../../static/js/calibration.js");
const JS_BLUETOOTH: &str = include_str!("../../static/js/bluetooth.js");
const JS_NETWORK: &str = include_str!("../../static/js/network.js");
const JS_KEYBOARD: &str = include_str!("../../static/js/keyboard.js");
const JS_CHEATSHEET: &str = include_str!("../../static/js/cheatsheet.js");
const JS_SHORTCUTS: &str = include_str!("../../static/js/shortcuts.js");
const JS_APP: &str = include_str!("../../static/js/app.js");

fn serve(stream: &mut std::net::TcpStream, content_type: &str, body: &str) {
    let response = format!(
        "HTTP/1.1 200 OK\r\nContent-Type: {}\r\nCache-Control: no-cache, no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
        content_type,
        body.len(),
        body
    );
    let _ = stream.write_all(response.as_bytes());
    let _ = stream.flush();
}

pub fn start(addr: &str) {
    let listener = TcpListener::bind(addr).unwrap();
    println!("Web UI available at http://{}", super::display_addr(addr));
    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            let mut request_line = String::new();
            let reader = BufReader::new(&stream);
            let mut first = true;
            for line in reader.lines() {
                match line {
                    Ok(line) if line.is_empty() => break,
                    Ok(line) => {
                        if first {
                            request_line = line;
                            first = false;
                        }
                    }
                    Err(_) => break,
                }
            }

            let path = request_line.split_whitespace().nth(1).unwrap_or("/");

            match path {
                "/style.css" => serve(&mut stream, "text/css", STYLE_CSS),
                "/css/tokens.css" => serve(&mut stream, "text/css", CSS_TOKENS),
                "/css/layout.css" => serve(&mut stream, "text/css", CSS_LAYOUT),
                "/css/components.css" => serve(&mut stream, "text/css", CSS_COMPONENTS),
                "/css/pages.css" => serve(&mut stream, "text/css", CSS_PAGES),
                "/js/websocket.js" => serve(&mut stream, "application/javascript", JS_WEBSOCKET),
                "/js/led-preview.js" => {
                    serve(&mut stream, "application/javascript", JS_LED_PREVIEW)
                }
                "/js/topbar.js" => serve(&mut stream, "application/javascript", JS_TOPBAR),
                "/js/navigation.js" => serve(&mut stream, "application/javascript", JS_NAVIGATION),
                "/js/animation.js" => serve(&mut stream, "application/javascript", JS_ANIMATION),
                "/js/audio-analysis.js" => {
                    serve(&mut stream, "application/javascript", JS_AUDIO_ANALYSIS)
                }
                "/js/visualizer.js" => serve(&mut stream, "application/javascript", JS_VISUALIZER),
                "/js/chromatic-circle.js" => {
                    serve(&mut stream, "application/javascript", JS_CHROMATIC_CIRCLE)
                }
                "/js/audio-capture.js" => {
                    serve(&mut stream, "application/javascript", JS_AUDIO_CAPTURE)
                }
                "/js/calibration.js" => {
                    serve(&mut stream, "application/javascript", JS_CALIBRATION)
                }
                "/js/bluetooth.js" => serve(&mut stream, "application/javascript", JS_BLUETOOTH),
                "/js/network.js" => serve(&mut stream, "application/javascript", JS_NETWORK),
                "/js/keyboard.js" => serve(&mut stream, "application/javascript", JS_KEYBOARD),
                "/js/cheatsheet.js" => {
                    serve(&mut stream, "application/javascript", JS_CHEATSHEET)
                }
                "/js/shortcuts.js" => {
                    serve(&mut stream, "application/javascript", JS_SHORTCUTS)
                }
                "/js/app.js" => serve(&mut stream, "application/javascript", JS_APP),
                // SPA fallback: all other paths get index.html
                _ => serve(&mut stream, "text/html", INDEX_HTML),
            }
        }
    }
}
