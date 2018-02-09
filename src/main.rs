extern crate futures_glib;
extern crate gtk;
extern crate gdk_pixbuf;
#[macro_use(s)]
extern crate ndarray;
#[macro_use]
extern crate relm;
#[macro_use]
extern crate relm_derive;
extern crate rscam;

use futures_glib::Timeout;
use gdk_pixbuf::{Pixbuf, PixbufLoader};
use gtk::{
    Button,
    ButtonExt,
    ContainerExt,
    Image,
    ImageExt,
    Inhibit,
    Label,
    LabelExt,
    WidgetExt,
    Window,
    WindowType,
};
use gtk::Orientation::Vertical;
use ndarray::{Array, Array2, arr2};
use relm::{Relm, Update, Widget};
use rscam::{Camera, Config};
use std::ops::Mul;
use std::time::Duration;

struct Model {
    relm: Relm<Win>,
    started_camera: Option<Camera>,
}

#[derive(Msg)]
enum Msg {
    ToggleCamera,
    Quit,
    UpdateCameraImage(()),
}

// Create the structure that holds the widgets used in the view.
struct Win {
    state_label: Label,
    color_image: Image,
    gray_image: Image,
    model: Model,
    window: Window,
}

fn jpeg_vec_to_pixbuf(jpeg_vec: &[u8]) -> Pixbuf {
    let loader = PixbufLoader::new();
    loader.loader_write(jpeg_vec).unwrap();
    loader.close().unwrap();
    loader.get_pixbuf().unwrap()
}

fn pixbuf_to_gray_mat(color_pixbuf: &Pixbuf) -> Array2<f32> {
    let mut gray_pixels: Vec<f32> = vec![];
    unsafe {
        for rgb in color_pixbuf.get_pixels().chunks(3) {
            let mut pixel: u16 = (0.3 * rgb[0] as f32 +
                                  0.59 * rgb[1] as f32 +
                                  0.11 * rgb[2] as f32) as u16;
            if pixel > 0xff as u16 {
                pixel = 0xff as u16;
            }
            gray_pixels.push(pixel as f32);
        }
    }
    let w = color_pixbuf.get_width() as usize;
    let h = color_pixbuf.get_height() as usize;
    Array::from_vec(gray_pixels).into_shape((h, w)).unwrap()
}

fn apply_gaussian_filter(gray_mat: Array2<f32>) -> Array2<f32> {
    let mut blur_vec: Vec<f32> = vec![];
    let edge_detect_mat = arr2(
        &[[2.,  4.,  5.,  4., 2.],
          [4.,  9., 12.,  9., 4.],
          [5., 12., 15., 12., 5.],
          [4.,  9., 12.,  9., 4.],
          [2.,  4.,  5.,  4., 2.]]);
    let base = edge_detect_mat.scalar_sum();
    let w: usize;
    let h: usize;
    {
        let shape = gray_mat.shape();
        h = shape[0];
        w = shape[1];
    }
    for i in 0..h {
        for j in 0..w {
            if i<3 || h-4<i || j<3 || w-4<j {
                blur_vec.push(*gray_mat.get((i, j)).unwrap());
            } else {
                let mat = gray_mat.slice(s![i-2..i+3, j-2..j+3]);
                let mat = mat.mul(&edge_detect_mat);
                let pixel = mat.scalar_sum() / base;
                blur_vec.push(pixel);
            }
        }
    }
    Array::from_vec(blur_vec).into_shape((h, w)).unwrap()
}

fn get_rough_angle(angle: f32) -> i32 {
    if ((angle > 22.5) && (angle < 67.5)) || ((angle < -112.5) && (angle > -157.5)) {
        45
    } else if ((angle > 67.5) && (angle < 112.5)) || ((angle < -67.5) && (angle > -112.5)) {
        90
    } else if ((angle > 112.5) && (angle < 157.5)) || ((angle < -22.5) && (angle > -67.5)) {
        135
    } else {
        0
    }
}

fn is_edge_pixel(current_x: usize, current_y: usize, image_w: usize, strength_vec: &Vec<f32>, compare_angle: i32) -> bool {
    let current_index = image_w * current_y + current_x;
    let compare_index1: usize;
    let compare_index2: usize;
    match compare_angle {
        45 => {
            compare_index1 = image_w * current_y + current_x + 1;
            compare_index2 = image_w * current_y + current_x - 1;
        },
        90 => {
            compare_index1 = image_w * (current_y - 1) + current_x - 1;
            compare_index2 = image_w * (current_y + 1) + current_x + 1;
        },
        135 => {
            compare_index1 = image_w * (current_y - 1) + current_x - 1;
            compare_index2 = image_w * (current_y + 1) + current_x + 1;
        },
        _ => {
            compare_index1 = image_w * (current_y - 1) + current_x;
            compare_index2 = image_w * (current_y + 1) + current_x;
        },
    }
    let current_strength = strength_vec[current_index];
    let compare_strength1 = strength_vec[compare_index1];
    let compare_strength2 = strength_vec[compare_index2];
    current_strength > compare_strength1 && current_strength > compare_strength2
}

fn get_edge(gray_mat: Array2<f32>) -> Array2<f32> {
    let mut edge_vec: Vec<f32> = vec![];
    let w: usize;
    let h: usize;
    {
        let shape = gray_mat.shape();
        h = shape[0];
        w = shape[1];
    }
    let gx_mat = arr2(&[[-1.,  0.,  1.],
                        [-2.,  0.,  2.],
                        [-1.,  0.,  1.]]);
    let gy_mat = arr2(&[[-1., -2., -1.],
                        [ 0.,  0.,  0.],
                        [ 1.,  2.,  1.]]);
    let mut strength_vec: Vec<f32> = vec![];
    let mut angle_vec: Vec<i32> = vec![];
    for i in 0..h {
        for j in 0..w {
            if i<2 || h-3<i || j<2 || w-3<j {
                strength_vec.push(0.);
                angle_vec.push(0);
            } else {
                let mat = gray_mat.slice(s![i-1..i+2, j-1..j+2]);
                let gx = mat.mul(&gx_mat).scalar_sum();
                let gy = mat.mul(&gy_mat).scalar_sum();
                let strength = (gx.powf(2.) + gy.powf(2.)).sqrt();
                strength_vec.push(strength);
                let raw_angle = ((gx/gy).atan() / std::f32::consts::PI) * 180.;
                angle_vec.push(get_rough_angle(raw_angle));
            }
        }
    }
    for i in 0..h {
        for j in 0..w {
            if i<2 || h-3<i || j<2 || w-3<j {
                edge_vec.push(0.);
            } else {
                let this_index = w * i + j;
                let result =
                    if strength_vec[this_index] > 10. && is_edge_pixel(j, i, w, &strength_vec, angle_vec[this_index]) {
                        255.
                    } else {
                        0.
                    };
                edge_vec.push(result);
            }
        }
    }
    Array::from_vec(edge_vec).into_shape((h, w)).unwrap()
}

fn mat_to_pixbuf(edge_mat: Array2<f32>) -> Pixbuf {
    let uw: usize;
    let uh: usize;
    {
        let shape = &edge_mat.shape();
        uh = shape[0];
        uw = shape[1];
    }
    let iw = uw as i32;
    let ih = uh as i32;
    let mut gray_rgb_vec:Vec<u8> = vec![];
    for p in edge_mat.into_shape((uw * uh)).unwrap().to_vec() {
        gray_rgb_vec.extend_from_slice(&[p as u8, p as u8, p as u8])
    }
    Pixbuf::new_from_vec(
        gray_rgb_vec,
        0, // pixbuf supports only RGB
        false,
        8,
        iw, ih, iw * 3)
}

impl Update for Win {
    // Specify the model used for this widget.
    type Model = Model;
    // Specify the model parameter used to init the model.
    type ModelParam = ();
    // Specify the type of the messages sent to the update function.
    type Msg = Msg;

    fn model(relm: &Relm<Self>, _: ()) -> Model {
        Model {
            relm: relm.clone(),
            started_camera: None,
        }
    }

    fn update(&mut self, event: Msg) {
        match event {
            Msg::ToggleCamera => {
                if self.model.started_camera.is_some() {
                    self.close_camera();
                } else {
                    self.open_camera();
                    self.set_msg_timeout(10, Msg::UpdateCameraImage);
                }
            },
            Msg::UpdateCameraImage(()) => {
                if self.model.started_camera.is_some() {
                    let color_pixbuf = self.update_camera_image();
                    if color_pixbuf.is_some() {
                        let color_pixbuf = color_pixbuf.unwrap();
                        self.set_msg_timeout(10, Msg::UpdateCameraImage);
                        let gray_mat = pixbuf_to_gray_mat(&color_pixbuf);
                        let blur_gray_mat = apply_gaussian_filter(gray_mat);
                        let edge_mat = get_edge(blur_gray_mat);
                        let pixbuf = mat_to_pixbuf(edge_mat);
                        let gray_image = &self.gray_image;
                        gray_image.set_from_pixbuf(&pixbuf);
                    }
                }
            },
            Msg::Quit => gtk::main_quit(),
        }
    }
}

impl Widget for Win {
    // Specify the type of the root widget.
    type Root = Window;

    // Return the root widget.
    fn root(&self) -> Self::Root {
        self.window.clone()
    }

    fn view(relm: &Relm<Self>, model: Self::Model) -> Self {
        // Create the view using the normal GTK+ method calls.
        let vbox = gtk::Box::new(Vertical, 0);

        let state_label = Label::new("wait to toggle camera");
        vbox.add(&state_label);

        let toggle_camera_button = Button::new_with_label("toggle camera");
        vbox.add(&toggle_camera_button);

        let color_image = Image::new();
        vbox.add(&color_image);

        let gray_image = Image::new();
        vbox.add(&gray_image);

        let window = Window::new(WindowType::Toplevel);
        window.add(&vbox);
        window.show_all();

        // Send the message Increment when the button is clicked.
        connect!(relm, toggle_camera_button, connect_clicked(_), Msg::ToggleCamera);
        connect!(relm, window, connect_delete_event(_, _), return (Some(Msg::Quit), Inhibit(false)));

        Win {
            state_label: state_label,
            color_image: color_image,
            gray_image: gray_image,
            model,
            window: window,
        }
    }
}

/*
fn print_pixbuf_info(pixbuf: &Pixbuf) {
    unsafe {
        println!("pixels len: {:?}", pixbuf.get_pixels().len());
    }
    println!("width: {:?}", pixbuf.get_width());
    println!("height: {:?}", pixbuf.get_height());
    println!("width x height: {:?}", pixbuf.get_width() * pixbuf.get_height());
    println!("rowstride: {:?}", pixbuf.get_rowstride());
    println!("colorspace: {:?}", pixbuf.get_colorspace());
    println!("has alpha: {:?}", pixbuf.get_has_alpha());
    println!("bits per sample: {:?}", pixbuf.get_bits_per_sample());
}
*/

impl Win {
    fn set_msg_timeout<CALLBACK>(
        &mut self,
        millis: u64,
        callback: CALLBACK,
    )
        where CALLBACK: Fn(()) -> Msg + 'static,
    {
        let stream = Timeout::new(Duration::from_millis(millis));
        self.model.relm.connect_exec_ignore_err(stream, callback);
    }

    fn update_camera_image(&mut self) -> Option<Pixbuf> {
        let camera = self.model.started_camera.as_mut().unwrap();
        let frame = camera.capture().unwrap();
        let pixbuf = jpeg_vec_to_pixbuf(&frame[..]);
        let color_image = &self.color_image;
        color_image.set_from_pixbuf(&pixbuf);
        while gtk::events_pending() {
            gtk::main_iteration_do(true);
        }
        Some(pixbuf)
    }

    fn open_camera(&mut self) {
        let label = &self.state_label;
        let mut camera = Camera::new("/dev/video0").unwrap();
        camera.start(&Config {
            interval: (1, 30), // 30 fps.
            resolution: (640, 360),
            format: b"MJPG",
            ..Default::default()
        }).unwrap();
        self.model.started_camera = Some(camera);
        label.set_text("opened camera");
    }

    fn close_camera(&mut self) {
        self.model.started_camera = None;
        let label = &self.state_label;
        label.set_text("closed camera");
    }
}

fn main() {
    Win::run(()).unwrap();
}
