use anyhow::Error;
use derive_more::{Display, Error};
use gst::{element_error, element_warning};
use gst::{prelude::*, Element, Pipeline};
use gtk::{gdk, glib};

use std::sync::{Arc, Mutex};
#[derive(Debug, Display, Error)]
#[display(fmt = "Received error from {src}: {error} (debug: {debug:?})")]
struct ErrorMessage {
    src: glib::GString,
    error: glib::Error,
    debug: Option<glib::GString>,
}

#[derive(Clone, Debug, glib::Boxed)]
#[boxed_type(name = "ErrorValue")]
struct ErrorValue(Arc<Mutex<Option<Error>>>);
#[derive(glib::Downgrade)]
pub struct GstManager {
    gtksink: Element,
    pipeline: Pipeline,
}

impl GstManager {
    pub fn new() -> Self {
        Self {
            gtksink: gst::ElementFactory::make("gtk4paintablesink")
                .build()
                .unwrap(),
            pipeline: gst::Pipeline::new(),
        }
    }

    pub fn get_pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    pub fn get_paintable_sink(&self) -> gdk::Paintable {
        self.gtksink.property::<gdk::Paintable>("paintable")
    }

    pub fn set_video_filename(&self, filename: Option<&str>) {
        self.build_pipeline(filename);
    }

    pub fn set_play_stream(&self) {
        self.pipeline
            .set_state(gst::State::Playing)
            .expect("Unable to set the pipeline to the `Playing` state");
    }
    pub fn set_pause_stream(&self) {
        self.pipeline
            .set_state(gst::State::Paused)
            .expect("Unable to set the pipeline to the `Paused` state");
    }

    pub fn set_stop_stream(&self) {
        self.pipeline
            .set_state(gst::State::Null)
            .expect("Unable to set the pipeline to the `Null` state");
    }

    fn build_pipeline(&self, filename: Option<&str>) {
        dbg!("{}", filename);
        let filesrc = gst::ElementFactory::make("filesrc")
            .name("filesrc")
            .property("location", filename)
            .build()
            .unwrap();
        let decodebin = gst::ElementFactory::make("decodebin")
            .name("decodebin")
            .build()
            .unwrap();

        let binsink = gst::Bin::with_name("binsink");
        let tee = gst::ElementFactory::make("tee")
            .name("tee")
            .build()
            .unwrap();

        let queue0 = gst::ElementFactory::make("queue")
            .name("queue0")
            .build()
            .unwrap();
        let videoconvert = gst::ElementFactory::make("videoconvert")
            .name("videoconvert")
            .build()
            .unwrap();
        binsink
            .add_many([&tee, &queue0, &videoconvert, &self.gtksink])
            .unwrap();

        gst::Element::link_many([&tee, &queue0, &videoconvert, &self.gtksink]).unwrap();
        binsink
            .add_pad(&gst::GhostPad::with_target(&tee.static_pad("sink").unwrap()).unwrap())
            .unwrap();

        let sink = binsink.upcast();

        self.pipeline
            .add_many([&filesrc, &decodebin, &sink])
            .unwrap();
        gst::Element::link_many([&filesrc, &decodebin]).unwrap();

        let pipeline_weak = self.pipeline.downgrade();
        let sink_weak = sink.downgrade();

        decodebin.connect_pad_added(move |dbin, src_pad| {
            let (is_audio, is_video) = {
                let media_type = src_pad.current_caps().and_then(|caps| {
                    caps.structure(0).map(|s| {
                        let name = s.name();
                        (name.starts_with("audio/"), name.starts_with("video/"))
                    })
                });

                match media_type {
                    None => {
                        element_warning!(
                            dbin,
                            gst::CoreError::Negotiation,
                            ("Failed to get media type from pad {}", src_pad.name())
                        );

                        return;
                    }
                    Some(media_type) => media_type,
                }
            };
            println!("new pad {:?}", src_pad);
            let pipeline = match pipeline_weak.upgrade() {
                Some(pipeline) => pipeline,
                None => return,
            };
            let sink = match sink_weak.upgrade() {
                Some(sink) => sink,
                None => return,
            };
            // let audiosink = match audiosink_weak.upgrade() {
            //     Some(audiosink) => audiosink,
            //     None => return,
            // };
            let insert_sink = |is_audio, is_video| -> Result<(), Error> {
                if is_audio {
                    let queue = gst::ElementFactory::make("queue")
                        .name("audioqueue")
                        .build()
                        .unwrap();
                    let convert = gst::ElementFactory::make("audioconvert")
                        .name("audioconvert")
                        .build()
                        .unwrap();
                    let resample = gst::ElementFactory::make("audioresample")
                        .name("audioresample")
                        .build()
                        .unwrap();
                    let sink = gst::ElementFactory::make("autoaudiosink")
                        .name("autoaudiosink")
                        .build()
                        .unwrap();

                    let elements = &[&queue, &convert, &resample, &sink];
                    pipeline.add_many(elements).unwrap();
                    gst::Element::link_many(elements).unwrap();

                    // !!ATTENTION!!:
                    // This is quite important and people forget it often. Without making sure that
                    // the new elements have the same state as the pipeline, things will fail later.
                    // They would still be in Null state and can't process data.
                    for e in elements {
                        e.sync_state_with_parent().unwrap();
                    }

                    // Get the queue element's sink pad and link the decodebin's newly created
                    // src pad for the audio stream to it.
                    let sink_pad = queue.static_pad("sink").expect("queue has no sinkpad");
                    src_pad.link(&sink_pad).expect("Audio Link Failed");
                } else if is_video {
                    pipeline.remove(&sink).unwrap();
                    pipeline.add(&sink).unwrap();
                    sink.sync_state_with_parent().unwrap();

                    let sink_pad = sink.static_pad("sink").unwrap();
                    src_pad.link(&sink_pad).unwrap();
                }
                Ok(())
            };

            if let Err(err) = insert_sink(is_audio, is_video) {
                // The following sends a message of type Error on the bus, containing our detailed
                // error information.
                element_error!(
                    dbin,
                    gst::LibraryError::Failed,
                    ("Failed to insert sink"),
                    details: gst::Structure::builder("error-details")
                                .field("error",
                                       &ErrorValue(Arc::new(Mutex::new(Some(err)))))
                                .build()
                );
            }
        });
    }
}
