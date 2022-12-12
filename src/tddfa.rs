use std::error::Error;
use crate::{model::OnnxSessionsManager, utils::{crop_img, parse_roi_box_from_bbox, parse_roi_box_from_landmark, _parse_param, similar_transform}};
use std::sync::Mutex;

use onnxruntime::{
    environment::Environment, session::Session, GraphOptimizationLevel, LoggingLevel, OrtError,
};
use serde::{Deserialize, Serialize};
use onnxruntime::tensor::OrtOwnedTensor;

use once_cell::sync::Lazy;
use opencv::{core::{Size, Vec3b}, imgproc, prelude::*, videoio, imgcodecs};


use std::{
    ops::Deref,
};
use onnxruntime::ndarray::{arr1, arr2, Array4, Array1, Array2, Axis, ArrayBase, OwnedRepr, Dim, Order, s};


#[derive(Serialize, Deserialize)]
struct DataStruct {
    mean: Vec<f32>,
    std: Vec<f32>,
    u_base: Vec<Vec<f32>>,
    w_shp_base: Vec<Vec<f32>>,
    w_exp_base: Vec<Vec<f32>>,
}


static ENVIRONMENT: Lazy<Environment> = Lazy::new(|| {
    OnnxSessionsManager::get_environment(&"Landmark Detection").unwrap()
});


pub struct TDDFA {
    data:DataStruct,
    landmark_model:Mutex<Session<'static>>,
    size:i32,
    mean_array:[f32;62],
    std_array:[f32;62],
    u_base_array:ArrayBase<OwnedRepr<f32>, Dim<[usize; 2]>>
    ,
    w_shp_base_array:ArrayBase<OwnedRepr<f32>, Dim<[usize; 2]>>
    ,
    w_exp_base_array:ArrayBase<OwnedRepr<f32>, Dim<[usize; 2]>>

}

impl TDDFA{
    pub fn new(
        // bfm_onnx_fp: &str,
        data_fp: &str,
        landmark_model_path: &str,
        size: i32,
        )-> Result<Self, Box<dyn Error>> { 


        let landmark_model = OnnxSessionsManager::initialize_model(&ENVIRONMENT, landmark_model_path.to_string(), 1)?;
        let landmark_model = Mutex::new(landmark_model);

        let data = {
            let data = std::fs::read_to_string(&data_fp).unwrap();
            serde_json::from_str::<DataStruct>(&data).unwrap()
        };

        
        let mean_array:[f32;62] = data.mean.as_slice().try_into().unwrap();
        let std_array:[f32;62] = data.std.as_slice().try_into().unwrap();

        
        let mut u_base_array = Array2::<f32>::default((204, 1));
        for (i, mut row) in u_base_array.axis_iter_mut(Axis(0)).enumerate() {
            for (j, col) in row.iter_mut().enumerate() {
                *col = data.u_base[i][j];
            }
        }

        let mut w_shp_base_array = Array2::<f32>::default((204, 40));
        for (i, mut row) in w_shp_base_array.axis_iter_mut(Axis(0)).enumerate() {
            for (j, col) in row.iter_mut().enumerate() {
                *col = data.w_shp_base[i][j];
            }
        }

        let mut w_exp_base_array = Array2::<f32>::default((204, 10));
        for (i, mut row) in w_exp_base_array.axis_iter_mut(Axis(0)).enumerate() {
            for (j, col) in row.iter_mut().enumerate() {
                *col = data.w_exp_base[i][j];
            }
        }


        Ok(Self {
            data,
            landmark_model,
            size,
            mean_array,
            std_array,
            u_base_array,
            w_shp_base_array,
            w_exp_base_array
        })
        }

        pub fn run(
            &self,
            input_frame: &Mat, face_box: [f32; 4], ver:Vec<Vec<f32>>, crop_policy:&str) -> Result<([f32;62], [f32; 4]), Box<dyn Error>> {

                let mut roi_box = [0.;4];
                if crop_policy == "box" {
                    // by face box
                    roi_box = parse_roi_box_from_bbox(face_box);
                } else if crop_policy == "landmark" {
                    // by landmarks
                    roi_box = parse_roi_box_from_landmark(ver);
                } else {
                    println!("Invalid crop policy")
                    // return Err(format!("Unknown crop policy {}", crop_policy));
                }



            let mut rgb_frame = Mat::default();
            imgproc::cvt_color(&input_frame, &mut rgb_frame, imgproc::COLOR_BGR2RGB, 0).unwrap();
            
            // let cropped_image = Mat::roi(&rgb_frame, opencv::core::Rect {
            //     x: roi_box[0].round() as i32,
            //     y: roi_box[1].round() as i32,    
            //     width: roi_box[2].round() as i32,  
            //     height: roi_box[3].round() as i32,
            // }).unwrap();
// println!("{:?}", roi_box);
            let cropped_image = crop_img(&rgb_frame, roi_box);

            // Resizing the frame
            let mut resized_frame = Mat::default();
            imgproc::resize(
                &cropped_image,
                &mut resized_frame,
                Size {
                    width: self.size,
                    height: self.size,
                },
                0.0,
                0.0,
                imgproc::INTER_LINEAR // INTER_AREA, // https://stackoverflow.com/a/51042104 | Speed -> https://stackoverflow.com/a/44278268
            ).unwrap();

            let vec = Mat::data_typed::<Vec3b>(&resized_frame).unwrap();

            let array = Array4::from_shape_fn((1, 3, self.size as usize, self.size as usize), |(_, c, y, x)| {
                (Vec3b::deref(&vec[x + y * self.size as usize])[c] as f32 - 127.5) / 128.0
            })
            .into();

            let input_tensor_values = vec![array];

            // // Inference
            let mut landmark_model = self.landmark_model.lock().unwrap();
            let param: Vec<OrtOwnedTensor<f32, _>> = landmark_model.run(input_tensor_values).unwrap();
            let param:[f32;62] = param[0].as_slice().unwrap().try_into().unwrap();

            

            let processed_param =  arr1(&param) * arr1(&self.std_array) + arr1(&self.mean_array);
            let processed_param:[f32;62] = processed_param.as_slice().unwrap().try_into().unwrap();
            // println!("{:?}", processed_param);
            Ok((processed_param, roi_box))

            }


        pub fn recon_vers(&self, param: [f32;62], roi_box:[f32; 4]) -> Vec<Vec<f32>>        { // -> ArrayBase<OwnedRepr<f32>, Dim<[usize; 2]>> 

                let (R, offset, alpha_shp, alpha_exp) = _parse_param(&param).unwrap();


                let pts3d = &self.u_base_array + (&self.w_shp_base_array.dot(&arr2(&alpha_shp))) + (&self.w_exp_base_array.dot(&arr2(&alpha_exp)));


                let pts3d = pts3d.to_shape(((3, 68), Order::ColumnMajor)).unwrap(); // Note : the numbers are in different orders into_shape((3, 68)).unwrap()
   

                let pts3d = arr2(&R).dot(&pts3d) + arr2(&offset);

                // pts3d.fla

                // let mut processed_pts3d = pts3d;
                let vec_pts_3d = vec![pts3d.slice(s![0, ..]).to_vec(), pts3d.slice(s![1, ..]).to_vec(), pts3d.slice(s![2, ..]).to_vec()];

                // println!("{:?}", pts_3d);
                let out_pts3d = similar_transform(vec_pts_3d, roi_box, self.size);

                
                out_pts3d
            }



        }

#[test]
pub fn test() {
    use std::time::Instant;


    let data_fp = "./assets/data.json";
    let landmark_model_path = "./assets/mb05_120x120.onnx";
    let size = 120;
  
    let bfm = TDDFA::new(
        // bfm_onnx_fp,
        data_fp,
        landmark_model_path,
        size,
     
        ).unwrap(); 

    let mut frame = Mat::default();
    imgcodecs::imread("test.jpg", 1)
        .map(|m| frame = m)
        .unwrap();

    // println!("{:?}", bfm.data.mean);

    // loop {
    //     let start_time = Instant::now();

    let face_box = [150., 150., 400., 400.];
        let (param, roi_box) = bfm.run(&frame, face_box, vec![vec![1., 2., 3.], vec![4., 5., 6.], vec![7., 8., 9.]], "box").unwrap();
        let pts_3d = bfm.recon_vers(param, roi_box);

        let (param, roi_box) = bfm.run(&frame, face_box, pts_3d, "landmark").unwrap();
        let pts_3d = bfm.recon_vers(param, roi_box);

    //     let elapsed_time = start_time.elapsed();
    //     println!("{} ms", elapsed_time.as_millis());
    // }
    
    // println!("{:?}", pts_3d);
    

    }