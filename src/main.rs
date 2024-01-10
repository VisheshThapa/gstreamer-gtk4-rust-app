mod gstmodule;
use gst::prelude::*;

use gstmodule::gstmanager::GstManager;
use gtk::{gio, glib};
use gtk::{prelude::*, Button};
use std::cell::RefCell;

fn create_ui(app: &gtk::Application) {
    let window = gtk::ApplicationWindow::new(app);
    let gstman = GstManager::new();

    window.set_default_size(640, 480);
    window.set_title(Some("GstPlayer - Gtk4 + Gstreamer"));

    let paintable = gstman.get_paintable_sink();

    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 10);
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let hboxtop = gtk::Box::new(gtk::Orientation::Horizontal, 10);
    let picture = gtk::Picture::new();
    let label = gtk::Label::new(Some("Position: 00:00:00"));

    picture.set_paintable(Some(&paintable));

    let text_view = gtk::Label::new(Some("Please, select a mpegts video file."));
    let fchooser = Button::with_label("Open File");

    fchooser.connect_clicked(
        glib::clone!(@weak gstman,@weak text_view,@weak window => move |_| {
            println!("Done");
            //let videos_filter = gtk::FileFilter::new();
            // videos_filter.add_suffix(Some("*.mpegts"));
            //videos_filter.set_name(Some("MPEGTS"));

            let dialog = gtk::FileChooserDialog::builder()
                .title("Open File")
                .action(gtk::FileChooserAction::Open)
                .modal(true)
                // .filter(&videos_filter)
                .build();
            dialog.add_button("Cancel", gtk::ResponseType::Cancel);
            dialog.add_button("Accept", gtk::ResponseType::Accept);
            dialog.set_transient_for(Some(&window));
            dialog.run_async(glib::clone!(@weak gstman,@weak text_view => move |obj,res|{
                match res {
                    gtk::ResponseType::Accept => {
                        let file = obj.file().unwrap();
                        let from_str = gio::File::uri(&file).replace("file:///","/");
                        text_view.set_label(&from_str);
                        print!("{}",from_str);

                        gstman.set_video_filename(Some(&text_view.label().to_string()));
                    },
                    _ => {}
                }
                obj.destroy();
            }));
        }),
    );

    hboxtop.append(&fchooser);
    hboxtop.append(&text_view);
    hboxtop.set_halign(gtk::Align::Center);
    hboxtop.set_margin_top(20);

    vbox.append(&hboxtop);
    vbox.append(&picture);
    vbox.append(&label);
    vbox.append(&hbox);

    let pipeline_weak = gstman.get_pipeline().downgrade();
    let bus = gstman.get_pipeline().bus().unwrap();
    // let _pipeline = RefCell::new(Some(gstman.getPipeline()));

    let play_button = Button::with_label("Play");
    play_button.connect_clicked(glib::clone!(@weak gstman => move |_| {
        eprintln!("Play");
        gstman.set_play_stream();
    }));
    let pause_button = Button::with_label("Pause");
    pause_button.connect_clicked(glib::clone!(@weak gstman => move |_| {
        eprintln!("Pause");
        gstman.set_pause_stream();
    }));

    let stop_button = Button::with_label("Stop");
    stop_button.connect_clicked(move |_| {
        eprintln!("Stop");
        gstman.set_stop_stream();
    });

    hbox.append(&play_button);
    hbox.append(&pause_button);
    hbox.append(&stop_button);
    hbox.set_halign(gtk::Align::Center);
    hbox.set_margin_bottom(20);

    window.set_child(Some(&vbox));
    window.show();

    app.add_window(&window);

    let timeout_id = glib::timeout_add_local(std::time::Duration::from_millis(500), move || {
        let pipeline = match pipeline_weak.upgrade() {
            Some(pipeline) => pipeline,
            None => return glib::ControlFlow::Continue,
        };

        let position = pipeline.query_position::<gst::ClockTime>();
        label.set_text(&format!("Position: {:.0}", position.display()));
        glib::ControlFlow::Continue
    });

    let app_weak = app.downgrade();
    let bus_watch = bus
        .add_watch_local(move |_, msg| {
            use gst::MessageView;

            let app = match app_weak.upgrade() {
                Some(app) => app,
                None => return glib::ControlFlow::Break,
            };

            match msg.view() {
                MessageView::Eos(..) => app.quit(),
                MessageView::Error(err) => {
                    println!(
                        "Error from {:?}: {} ({:?})",
                        err.src().map(|s| s.path_string()),
                        err.error(),
                        err.debug()
                    );
                    app.quit();
                }
                _ => (),
            };

            glib::ControlFlow::Continue
        })
        .expect("Failed to add bus watch");

    let timeout_id = RefCell::new(Some(timeout_id));
    let bus_watch = RefCell::new(Some(bus_watch));
    app.connect_shutdown(move |_| {
        window.close();

        drop(bus_watch.borrow_mut().take());
        // if let Some(_pipeline) = _pipeline.borrow_mut().take() {
        // gstman.setStopStream();
        // }

        if let Some(timeout_id) = timeout_id.borrow_mut().take() {
            timeout_id.remove();
        }
    });
}

fn main() -> glib::ExitCode {
    std::env::set_var("RUST_BACKTRACE", "1");
    std::env::set_var("GST_DEBUG", "3");

    gst::init().unwrap();
    gtk::init().unwrap();

    gstgtk4::plugin_register_static().expect("Failed to register gstgtk4 plugin");

    let app = gtk::Application::new(None::<&str>, gio::ApplicationFlags::FLAGS_NONE);

    app.connect_activate(create_ui);
    let res = app.run();

    unsafe {
        gst::deinit();
    }

    res
}
