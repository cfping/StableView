use std::{
    sync::atomic::Ordering,
    thread,
    time::{Duration, Instant},
};

use iced::{
    application, executor, theme, widget::Container, Application, Color, Command, Element, Length,
    Subscription, Theme,
};
use iced_native::{mouse, window, Event};

use crate::{
    enums::message::Message,
    filter::EuroDataFilter,
    structs::{app::HeadTracker, state::AppConfig},
    structs::{camera::ThreadedCamera, network::SocketNetwork, pose::ProcessHeadPose},
};

use crate::gui::view::run_page;

use crate::consts::{APP_NAME, APP_REPOSITORY};

impl Application for HeadTracker {
    type Executor = executor::Default;
    type Flags = HeadTracker;
    type Message = Message;
    type Theme = Theme;

    fn new(flags: HeadTracker) -> (HeadTracker, Command<Message>) {
        (flags, Command::none())
    }

    fn title(&self) -> String {
        String::from(APP_NAME)
    }

    fn subscription(&self) -> Subscription<Message> {
        match self.config.hide_camera {
            true => iced_native::subscription::events().map(Message::EventOccurred),
            false => {
                if self.headtracker_running.load(Ordering::SeqCst) {
                    let ticks = iced::time::every(Duration::from_millis(1)).map(|_| Message::Tick);
                    let runtime_events =
                        iced_native::subscription::events().map(Message::EventOccurred);
                    Subscription::batch(vec![runtime_events, ticks])
                } else {
                    iced_native::subscription::events().map(Message::EventOccurred)
                }
            }
        }
    }

    // fn should_exit(&self) -> bool {
    // self.should_exit
    // }

    fn theme(&self) -> Theme {
        Theme::Light
    }

    fn style(&self) -> theme::Application {
        fn dark_background(_theme: &Theme) -> application::Appearance {
            application::Appearance {
                background_color: Color::from_rgb8(245, 245, 245),
                text_color: Color::BLACK,
            }
        }

        theme::Application::from(dark_background as fn(&Theme) -> _)
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Toggle => {
                if !self.headtracker_running.load(Ordering::SeqCst) {
                    self.headtracker_running.store(true, Ordering::SeqCst);

                    let camera_index = match self.camera_list.get(&self.config.selected_camera) {
                        Some(index) => *index,
                        // ! Should this be 0 or something else ?
                        None => {
                            tracing::error!("Unable to find camera index, setting default to 0");
                            0
                        }
                    };
                    let camera_name = self.config.selected_camera.clone();

                    let config = self.config.clone();

                    let headtracker_running = self.headtracker_running.clone();

                    let tx = self.sender.clone();
                    let rx = self.receiver.clone();

                    let error_tracker = self.error_tracker.clone();

                    self.headtracker_thread = Some(thread::spawn(move || {
                        let mut error_message = String::new();

                        // Resetting error message
                        {
                            let mut error_guard = error_tracker.lock().unwrap();
                            *error_guard = error_message.clone();
                        }

                        'inner: {
                            let mut euro_filter = EuroDataFilter::new(
                                config.min_cutoff.load(Ordering::SeqCst),
                                config.beta.load(Ordering::SeqCst),
                            );

                            let mut socket_network =
                                match SocketNetwork::new(config.ip.clone(), config.port.clone()) {
                                    Ok(socket) => socket,
                                    Err(error) => {
                                        error_message = error.to_string();
                                        tracing::error!(error_message);
                                        break 'inner;
                                    }
                                };

                            // Create a channel to communicate between threads
                            let mut thr_cam = match ThreadedCamera::start_camera_thread(
                                tx,
                                camera_index,
                                camera_name,
                            ) {
                                Ok(camera) => camera,
                                Err(error) => {
                                    error_message = error.to_string();
                                    tracing::error!(error_message);
                                    break 'inner;
                                }
                            };

                            let mut head_pose = match ProcessHeadPose::new(120) {
                                Ok(pose) => pose,
                                Err(error) => {
                                    error_message = error.to_string();
                                    tracing::error!(error_message);
                                    break 'inner;
                                }
                            };

                            let mut frame = match rx.recv() {
                                Ok(result) => result,
                                Err(error) => {
                                    error_message =
                                        format!("Unable to receive image data: {}", error);
                                    tracing::error!(error_message);
                                    opencv::core::Mat::default()
                                }
                            };
                            let mut data;

                            while headtracker_running.load(Ordering::SeqCst) {
                                let start_time = Instant::now();

                                frame = match rx.try_recv() {
                                    Ok(result) => result,
                                    Err(_) => frame.clone(),
                                };

                                let out = head_pose.single_iter(&frame);

                                match out {
                                    Ok(value) => {
                                        data = value;
                                    }
                                    Err(_) => {
                                        // println!("An error: {}; skipped.", e);
                                        // head_pose.face_box =  [150., 150., 400., 400.];
                                        // head_pose.pts_3d =
                                        //     vec![vec![1., 2., 3.], vec![4., 5., 6.], vec![7., 8., 9.]];
                                        // head_pose.face_box = [0., 0., 600., 600.];
                                        // headtracker_running.store(false, Ordering::SeqCst);
                                        continue;
                                    }
                                };

                                data = euro_filter.filter_data(
                                    data,
                                    Some(config.min_cutoff.load(Ordering::SeqCst)),
                                    Some(config.beta.load(Ordering::SeqCst)),
                                );

                                match socket_network.send(data) {
                                    Ok(_) => {}
                                    Err(_) => {
                                        error_message = format!(
                                            "Unable to send data to {}:{}",
                                            &config.ip, &config.port
                                        );
                                        tracing::error!(error_message);
                                        break;
                                    }
                                };

                                let elapsed_time = start_time.elapsed();
                                let delay_time = ((1000 / config.fps.load(Ordering::SeqCst))
                                    as f32
                                    - elapsed_time.as_millis() as f32)
                                    .max(0.);
                                thread::sleep(Duration::from_millis(delay_time.round() as u64));
                            }

                            thr_cam.shutdown();
                        }

                        let mut error_guard = error_tracker.lock().unwrap();
                        *error_guard = String::from(error_message);
                        headtracker_running.store(false, Ordering::SeqCst);
                    }));
                } else {
                    self.headtracker_running.store(false, Ordering::SeqCst);

                    match self.headtracker_thread.take() {
                        Some(thread) => match thread.join() {
                            Ok(_) => {}
                            Err(e) => tracing::error!("Could not join spawned thread: {:?}", e),
                        },
                        None => tracing::error!("Called stop on non-running thread"),
                    }
                }
            }
            Message::Tick => {
                self.frame = match self.receiver.try_recv() {
                    Ok(result) => result,
                    Err(_) => self.frame.clone(),
                };
            }
            Message::MinCutoffSliderChanged(value) => {
                if value == 0 {
                    self.config.min_cutoff.store(0., Ordering::SeqCst)
                } else {
                    self.config
                        .min_cutoff
                        .store(1. / ((value * value) as f32), Ordering::SeqCst)
                };
                self.save_config()
            }
            Message::BetaSliderChanged(value) => {
                if value == 0 {
                    self.config.beta.store(0., Ordering::SeqCst)
                } else {
                    self.config
                        .beta
                        .store(1. / ((value * value) as f32), Ordering::SeqCst)
                };
                self.save_config()
            }
            Message::FPSSliderChanged(fps) => {
                self.config.fps.store(fps, Ordering::SeqCst);
                self.save_config()
            }
            Message::InputIP(ip) => {
                self.config.ip = ip;
                self.save_config()
            } // ! Input validation, four decimal with respective numbers between
            Message::InputPort(port) => {
                self.config.port = port;
                self.save_config()
            } // ! Input validation, only numbers

            Message::Camera(camera_name) => {
                self.config.selected_camera = camera_name;

                // If camera changes while running
                if self.headtracker_running.load(Ordering::SeqCst) {
                    // Turn it back off and on again :)
                    #[allow(unused_must_use)]
                    {
                        self.update(Message::Toggle);
                        self.update(Message::Toggle);
                    }
                }

                self.save_config()
            }
            Message::HideCamera(value) => {
                self.config.hide_camera = value;
                self.save_config()
            }
            // ! Need more asthetic default settings
            Message::DefaultSettings => {
                self.config
                    .min_cutoff
                    .store(AppConfig::default().min_cutoff, Ordering::SeqCst);
                self.config
                    .beta
                    .store(AppConfig::default().beta, Ordering::SeqCst);
                self.config
                    .fps
                    .store(AppConfig::default().fps, Ordering::SeqCst);
                self.config.ip = AppConfig::default().ip;
                self.config.port = AppConfig::default().port;
                self.config.hide_camera = AppConfig::default().hide_camera;

                self.save_config();
            }
            Message::OpenGithub => {
                #[cfg(target_os = "windows")]
                let program = "explorer";
                #[cfg(target_os = "macos")]
                let program = "open";
                #[cfg(target_os = "linux")]
                let program = "xdg-open";

                match std::process::Command::new(program)
                    .arg(APP_REPOSITORY)
                    .spawn()
                {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("Unable to open github repository : {:?}", e);

                        let mut error_guard = self.error_tracker.lock().unwrap();
                        *error_guard = String::from("Unable to open github repository");
                    }
                }
            }

            Message::OpenLogs => {
                #[cfg(target_os = "windows")]
                let program = "explorer";
                #[cfg(target_os = "macos")]
                let program = "open";
                #[cfg(target_os = "linux")]
                let program = "xdg-open";

                match std::process::Command::new(program)
                    .arg(
                        directories::ProjectDirs::from("rs", "", APP_NAME)
                            .unwrap()
                            .data_dir()
                            .to_str()
                            .unwrap(),
                    )
                    .spawn()
                {
                    Ok(_) => {}
                    Err(e) => {
                        tracing::error!("Unable to open logs directory : {:?}", e);

                        let mut error_guard = self.error_tracker.lock().unwrap();
                        *error_guard = String::from("Unable to open logs directory");
                    }
                }
            }
            Message::EventOccurred(event) => {
                if let Event::Window(window::Event::CloseRequested) = event {
                    if self.headtracker_running.load(Ordering::SeqCst) {
                        self.headtracker_running.store(false, Ordering::SeqCst);
                        match self.headtracker_thread.take() {
                            Some(thread) => match thread.join() {
                                Ok(_) => {}
                                Err(e) => {
                                    tracing::error!("Could not join spawned thread: {:?}", e);
                                }
                            },
                            None => {
                                tracing::error!("Called stop on non-running thread");
                            }
                        }
                    }
                    std::process::exit(0);
                }
                if let Event::Mouse(mouse::Event::ButtonPressed(mouse::Button::Left)) = event {
                    let mut error_guard = self.error_tracker.lock().unwrap();
                    *error_guard = String::new();

                    // Updating camera list
                    match ThreadedCamera::get_available_cameras() {
                        Ok(camera_list) => self.camera_list = camera_list,
                        Err(e) => {
                            tracing::error!("{}", e);
                            *error_guard = e.to_string();
                        }
                    }
                }
            }
        }
        Command::none()
    }

    fn view(&self) -> Element<Message> {
        let body = run_page(self);

        Container::new(body)
            .width(Length::Fill)
            .height(Length::Fill)
            .center_x()
            .center_y()
            .into()
    }
}
