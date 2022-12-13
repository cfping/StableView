mod model;
mod filter;
mod utils;
mod tddfa;
use std::{
    ops::Deref,
};
use model::OnnxSessionsManager;
use onnxruntime::ndarray::{Array, Array4};
use onnxruntime::tensor::OrtOwnedTensor;
use opencv::{core::{Size, Vec3b}, highgui, imgproc, prelude::*, videoio, imgcodecs};
use tddfa::TDDFA;
use std::time::Instant;

use crate::filter::{OneEuroFilter, DataFilters};
use crate::utils::{calc_pose, gen_point2d};


use std::net::{IpAddr, Ipv4Addr, SocketAddr, UdpSocket};
use std::mem::transmute;


use nokhwa::{Camera, CameraFormat};

use std::{thread, time::Duration, sync::mpsc};

fn main() -> Result<(), Box<dyn std::error::Error>> {

    let FPS_NEEDED = 60;

    // let mut euro_filter = OneEuroFilter::new(0.0, 1., 0.0, 0.0025, 0.01, 1.0);
    let mut euro_filters = DataFilters::new();
    // Create a channel to communicate between threads
    let (tx, rx) = mpsc::channel();


    let address = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 4242);
    let mut buf = [0; 8 * 6];
    let socket_network = UdpSocket::bind("0.0.0.0:0").expect("failed to bind UDP socket");


    let env = OnnxSessionsManager::get_environment(&"Landmark Detection").unwrap();
    let mut model =
    OnnxSessionsManager::initialize_model(&env, "./assets/mb05_120x120.onnx".to_string(), 1).unwrap();



    // let window = "video capture";
    // highgui::named_window(window, highgui::WINDOW_AUTOSIZE).unwrap();

    let dev = nokhwa::query_devices(nokhwa::CaptureAPIBackend::Auto).unwrap();
    println!("Available devices : ");

    for device_info in dev {
        println!("{} @ index {}", device_info.human_name(), device_info.index());
    }

    let mut cam = videoio::VideoCapture::new(0, videoio::CAP_ANY).unwrap(); // videoio::CAP_ANY, CAP_V4L2, // 0 is the default camera
    let opened = videoio::VideoCapture::is_opened(&cam).unwrap();

    if !opened {
        panic!("Unable to open default camera!");

        // ! In linux, query devices shows two different indexes for same device
        // ! If unable of open the 0th index, maybe try the other index also
    }



  // Spawn a thread to read frames from the camera
  let frame_reader = thread::spawn(move || {
    loop {
        // Reading frame
        let mut frame = Mat::default();
        cam.read(&mut frame).unwrap();

        // Send the frame to the other thread for processing
        tx.send(frame).unwrap();
    }
});

// Spawn a thread to run tddfa.run and tddfa.recon_vers on the frames received from the other thread
let tddfa_runner = thread::spawn(move || {



    let tddfa = TDDFA::new(
        // bfm_onnx_fp,
        "./assets/data.json",
        "./assets/mb05_120x120.onnx",
        120,
     
        ).unwrap(); 
    let face_box = [150., 150., 400., 400.];

    // Reading frame
    // let mut frame = Mat::default();
    // cam.read(&mut frame).unwrap();
    // Receive a frame from the other thread
    let mut frame = rx.recv().unwrap();



    let (param, roi_box) = tddfa.run(&frame, face_box, vec![vec![1., 2., 3.], vec![4., 5., 6.], vec![7., 8., 9.]], "box").unwrap();
    let pts_3d = tddfa.recon_vers(param, roi_box);

    
    let (param, roi_box) = tddfa.run(&frame, face_box, pts_3d, "landmark").unwrap();
    let pts_3d = tddfa.recon_vers(param, roi_box);

    loop {
        let start_time = Instant::now();
        
        // Reading frame
        // let mut frame = Mat::default();
        // cam.read(&mut frame).unwrap();
        // Receive a frame from the other thread

        // let mut frame:Mat;

        // println!("{:?}", rx.try_recv().is_err());
        // let received = rx.try_recv(); // .is_err();
        // println!("{:?}", received);
        // if received.is_err() {
        //     continue;
        // }
        
        // else{frame = rx}
        frame = match rx.try_recv() {
            Ok(result) => result,
            Err(_) => frame.clone()
        };


        // If frame is valid
        if frame.size().unwrap().width > 0 {

            let (param, roi_box) = tddfa.run(&frame, face_box, pts_3d.clone(), "landmark").unwrap();
            let pts_3d = tddfa.recon_vers(param, roi_box); // ? Commenting this code still seems to output the pose perfectly
                        
            let (P, pose) = calc_pose(&param);
            // println!("\r{:?}", pose);
            
            let (point2d, distance) = gen_point2d(&P, pts_3d);
            let distance = distance + (pose[0] * 0.2).abs();

            // let x = [point2d[0][0], point2d[1][0], point2d[2][0], point2d[3][0]];
            // let y = [point2d[0][1], point2d[1][1], point2d[2][1], point2d[3][1]];
            // let mut centroid = [x.iter().sum::<f32>() / 4., y.iter().sum::<f32>() / 4.];
            // * disbling the multiplying pose with distance (pose[0]*(distance/31), pose[1]*(distance/27)), it seems to causing jitting even when blinking eyes or smiling 
            // centroid[0] = centroid[0] + pose[0]; // * When very close to the camera, the head pose invariant seems to does't work, to miltgate the issue, we use this 
            // centroid[1] = centroid[1] - pose[1]*1.7; // * 31 & 27 represent the distance where head pose invariant is fully solved, and we use this ratio to make it work for closer distance
            // if pose[2] > 0.{centroid[0] = centroid[0] + pose[2].abs()}
            // else{centroid[0] = centroid[0] - pose[2].abs()}

            let mut data = [0.,0.,0., pose[0] as f64, -pose[1] as f64, pose[2] as f64];

            // data = euro_filter.filter_data(data);
            data = euro_filters.filter_data(data);

            unsafe {
                let ptr = buf.as_mut_ptr() as *mut [f64; 6];
                *ptr = data;
            }
            socket_network.send_to(&buf, &address).expect("failed to send data");

 
            // Showing the output
            // highgui::imshow(window, &mut frame)?;


        let elapsed_time = start_time.elapsed();
        // println!("{} ms", elapsed_time.as_millis());
        
        thread::sleep(Duration::from_millis(((1000/FPS_NEEDED)-elapsed_time.as_millis()).try_into().unwrap()));
        // let fps = cam.get(opencv::videoio::CAP_PROP_FPS).unwrap();
        // println!("{}x{} @ {} FPS @ {} ms", frame.cols(), frame.rows(), fps, elapsed_time.as_millis());

        }

        // let key = highgui::wait_key(10).unwrap();
        // if key > 0 && key != 255 {
        //     break;
        // }

    }});


        // Wait for the threads to finish
        frame_reader.join().unwrap();
        tddfa_runner.join().unwrap();
    

    
    Ok(())
}
