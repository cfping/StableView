use iced_native::Event;

#[derive(Debug, Clone)]
pub enum Message {
    Toggle,
    MinCutoffSliderChanged(u32),
    BetaSliderChanged(u32),
    InputIP(String),
    Camera(String),
    OpenGithub,
    OpenLogs,
    EventOccurred(Event),
}