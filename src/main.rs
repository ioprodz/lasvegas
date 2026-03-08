mod hardware;
mod web;

fn main() {
    // Hardware
    let controller = &mut hardware::led::create_controller();
    hardware::led::startup_animation(controller);

    // Web UI (runs in background thread)
    std::thread::spawn(|| web::http::start("0.0.0.0:3000"));

    // WebSocket server (blocks on main thread)
    web::websocket::start("0.0.0.0:8080", controller);
}
