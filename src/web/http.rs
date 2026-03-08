use std::io::{BufRead, BufReader, Write};
use std::net::TcpListener;

const INDEX_HTML: &str = include_str!("../../static/index.html");

pub fn start(addr: &str) {
    let listener = TcpListener::bind(addr).unwrap();
    println!("Web UI available at http://{}", addr);
    for stream in listener.incoming() {
        if let Ok(mut stream) = stream {
            // Read the request before responding
            let reader = BufReader::new(&stream);
            for line in reader.lines() {
                match line {
                    Ok(line) if line.is_empty() => break, // end of headers
                    Ok(_) => {}
                    Err(_) => break,
                }
            }

            let response = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: text/html\r\nCache-Control: no-cache, no-store\r\nConnection: close\r\nContent-Length: {}\r\n\r\n{}",
                INDEX_HTML.len(),
                INDEX_HTML
            );
            let _ = stream.write_all(response.as_bytes());
            let _ = stream.flush();
        }
    }
}
