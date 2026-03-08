use std::io::Write;
use std::net::TcpListener;

const INDEX_HTML: &str = include_str!("../../static/index.html");

pub fn start(addr: &str) {
    let listener = TcpListener::bind(addr).unwrap();
    println!("Web UI available at http://{}", addr);
    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nContent-Length: {}\r\n\r\n{}",
                INDEX_HTML.len(),
                INDEX_HTML
            );
            let _ = stream.write_all(response.as_bytes());
        }
    }
}
