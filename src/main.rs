extern crate futures_glib;
extern crate gtk;
extern crate gdk_pixbuf;
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
use ndarray::{Array, Array2};
use relm::{Relm, Update, Widget};
use rscam::{Camera, Config};
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

fn pixbuf_to_gray_mat(color_pixbuf: &Pixbuf) -> Array2<u8> {
    let mut gray_pixels: Vec<u8> = vec![];
    unsafe {
        for rgb in color_pixbuf.get_pixels().chunks(3) {
            let mut pixel: u16 = (0.3 * rgb[0] as f32 +
                                  0.59 * rgb[1] as f32 +
                                  0.11 * rgb[2] as f32) as u16;
            if pixel > 0xff as u16 {
                pixel = 0xff as u16;
            }
            gray_pixels.push(pixel as u8);
        }
    }
    let w = color_pixbuf.get_width() as usize;
    let h = color_pixbuf.get_height() as usize;
    Array::from_vec(gray_pixels).into_shape((w, h)).unwrap()
}

fn get_edge_mat(gray_mat: Array2<u8>) -> Array2<u8> {
    gray_mat
}

fn mat_to_pixbuf(edge_mat: Array2<u8>) -> Pixbuf {
    let uw: usize;
    let uh: usize;
    {
        let shape = &edge_mat.shape();
        uw = shape[0];
        uh = shape[1];
    }
    let iw = uw as i32;
    let ih = uh as i32;
    let mut gray_rgb_vec:Vec<u8> = vec![];
    for p in edge_mat.into_shape((uw * uh)).unwrap().to_vec() {
        gray_rgb_vec.extend_from_slice(&[p, p, p])
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
                        let edge_mat = get_edge_mat(gray_mat);
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
