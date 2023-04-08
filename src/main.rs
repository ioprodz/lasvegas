use rs_ws281x::ControllerBuilder;
use rs_ws281x::ChannelBuilder;
use rs_ws281x::StripType;

fn main() {
    // Construct a single channel controller. Note that the
    // Controller is initialized by default and is cleaned up on drop

    let controller = create_led_controller();
    render_color([255, 0, 50, 0], controller);

}

fn render_color(color: [u8; 4], mut controller: rs_ws281x::Controller) -> () {

    let leds = controller.leds_mut(0);
    for led in leds {
        *led = color;
    }
    controller.render().unwrap();
}

fn create_led_controller() -> rs_ws281x::Controller {
    return ControllerBuilder::new()
        .freq(800_000)
        .dma(10)
        .channel(
            0, // Channel Index
            ChannelBuilder::new()
            .pin(18) // GPIO 10 = SPI0 MOSI
            .count(60*6) // Number of LEDs
            .strip_type(StripType::Ws2811Gbr)
            .brightness(25) // default: 255
            .build(),
            )
        .build()
        .unwrap();
}

