use std::{
    sync::{atomic::Ordering, mpsc},
    thread,
};

use iced::{
    executor, widget::Container, Application, Command, Element, Length, Subscription, Theme,
};
use iced_native::{window, Event};

use crate::{
     enums::message::Message, filter::EuroDataFilter,
    structs::{camera::ThreadedCamera, network::SocketNetwork, pose::ProcessHeadPose},  structs::app::HeadTracker,
};

use crate::gui::view::run_page;

use super::style::{APP_NAME, APP_REPOSITORY};

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
        iced_native::subscription::events().map(Message::EventOccurred)
    }

    fn should_exit(&self) -> bool {
        self.should_exit
    }

    fn update(&mut self, message: Message) -> Command<Message> {
        match message {
            Message::Toggle => {
                if !self.keep_running.load(Ordering::SeqCst) {
                    self.keep_running.store(true, Ordering::SeqCst);
                    let cloned_keep_running = self.keep_running.clone();
                    let cloned_min_cutoff = self.min_cutoff.clone();
                    let cloned_beta = self.beta.clone();
                    let camera_index = *self.camera_list.get(self.selected_camera.as_ref().unwrap()).unwrap();
                    let cloned_cfg = self.cfg.clone();

                    thread::spawn(move || {
                        let mut euro_filter =
                            EuroDataFilter::new(cloned_cfg.min_cutoff, cloned_cfg.beta);
                        let mut socket_network =
                            SocketNetwork::new(cloned_cfg.ip_addr, cloned_cfg.port);

                        // Create a channel to communicate between threads
                        let (tx, rx) = mpsc::channel();
                        let mut thr_cam = ThreadedCamera::start_camera_thread(
                            tx,
                            camera_index,
                        );

                        let mut head_pose = ProcessHeadPose::new(120, 60);

                        let mut frame = rx.recv().unwrap();
                        let mut data;

                        while cloned_keep_running.load(Ordering::SeqCst) {
                            frame = match rx.try_recv() {
                                Ok(result) => result,
                                Err(_) => frame.clone(),
                            };

                            data = head_pose.single_iter(&frame);

                            data = euro_filter.filter_data(
                                data,
                                Some(cloned_min_cutoff.load(Ordering::SeqCst)),
                                Some(cloned_beta.load(Ordering::SeqCst)),
                            );

                            socket_network.send(data);
                        }

                        thr_cam.shutdown();
                    });
                } else {
                    self.keep_running.store(false, Ordering::SeqCst)
                    // println!("Thread already running")
                }
            }
            Message::MinCutoffSliderChanged(value) => self
                .min_cutoff
                .store(value as f32 / 10000., Ordering::SeqCst),
            Message::BetaSliderChanged(value) => {
                self.beta.store(value as f32 / 1000., Ordering::SeqCst)
            }

            Message::OpenGithub => {
                #[cfg(target_os = "windows")]
                std::process::Command::new("explorer")
                    .arg(APP_REPOSITORY)
                    .spawn()
                    .unwrap();
                #[cfg(target_os = "macos")]
                std::process::Command::new("open")
                    .arg(APP_REPOSITORY)
                    .spawn()
                    .unwrap();
                #[cfg(target_os = "linux")]
                std::process::Command::new("xdg-open")
                    .arg(APP_REPOSITORY)
                    .spawn()
                    .unwrap();
            }
            Message::OpenLogs => {
                #[cfg(target_os = "windows")]
                std::process::Command::new("explorer")
                    .arg(
                        directories::ProjectDirs::from("rs", "", APP_NAME)
                            .unwrap()
                            .data_dir()
                            .to_str()
                            .unwrap(),
                    )
                    .spawn()
                    .unwrap();
                #[cfg(target_os = "macos")]
                std::process::Command::new("open")
                    .arg("-t")
                    .arg(
                        directories::ProjectDirs::from("rs", "", APP_NAME)
                            .unwrap()
                            .data_dir()
                            .to_str()
                            .unwrap(),
                    )
                    .spawn()
                    .unwrap();
                #[cfg(target_os = "linux")]
                std::process::Command::new("xdg-open")
                    .arg(
                        directories::ProjectDirs::from("rs", "", APP_NAME)
                            .unwrap()
                            .data_dir()
                            .to_str()
                            .unwrap(),
                    )
                    .spawn()
                    .unwrap();
            }
            Message::InputIP(value) => println!("{value}"),
            Message::Camera(camera_name) => {self.selected_camera = Some(camera_name) },
            Message::EventOccurred(event) => {
                if Event::Window(window::Event::CloseRequested) == event {
                    self.keep_running.store(false, Ordering::SeqCst);
                    // thread::sleep(Duration::from_millis(100));
                    confy::store(APP_NAME, "config", self.cfg.clone()).unwrap();
                    self.should_exit = true;
          
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
