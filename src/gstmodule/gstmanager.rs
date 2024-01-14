use anyhow::Error;
use core::option::Option;
use derive_more::{Display, Error};
use gst::{element_error, element_warning};
use gst::{prelude::*, Bin, Element, Pipeline};
use gtk::{gdk, glib};
use std::cell::Cell;
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
    filesrc: Element,
    decodebin: Element,
    gtksink: Element,
    pipeline: Pipeline,
    binsink: Bin,
    tee: Element,
    queue0: Element,
    videoconvert: Element,
    audioqueue: Element,
    audioconvert: Element,
    audioresample: Element,
    audiosink: Element,
    added: Arc<Cell<bool>>,
}

impl GstManager {
    pub fn new() -> Self {
        Self {
            filesrc: gst::ElementFactory::make("filesrc")
                .name("filesrc")
                .build()
                .unwrap(),
            decodebin: gst::ElementFactory::make("decodebin")
                .name("decodebin")
                .build()
                .unwrap(),
            gtksink: gst::ElementFactory::make("gtk4paintablesink")
                .build()
                .unwrap(),
            pipeline: gst::Pipeline::new(),
            binsink: gst::Bin::with_name("binsink"),
            tee: gst::ElementFactory::make("tee")
                .name("tee")
                .build()
                .unwrap(),
            queue0: gst::ElementFactory::make("queue")
                .name("queue0")
                .build()
                .unwrap(),
            videoconvert: gst::ElementFactory::make("videoconvert")
                .name("videoconvert")
                .build()
                .unwrap(),
            audioqueue: gst::ElementFactory::make("queue")
                .name("audioqueue")
                .build()
                .unwrap(),
            audioconvert: gst::ElementFactory::make("audioconvert")
                .name("audioconvert")
                .build()
                .unwrap(),
            audioresample: gst::ElementFactory::make("audioresample")
                .name("audioresample")
                .build()
                .unwrap(),
            audiosink: gst::ElementFactory::make("autoaudiosink")
                .name("autoaudiosink")
                .build()
                .unwrap(),
            added: Arc::new(Cell::new(false)),
        }
    }

    pub fn get_pipeline(&self) -> &Pipeline {
        &self.pipeline
    }

    pub fn get_paintable_sink(&self) -> gdk::Paintable {
        self.gtksink.property::<gdk::Paintable>("paintable")
    }

    pub fn set_video_filename(&self, filename: Option<&str>) {
        if self.added.get() {
            self.reset();
        }
        self.run_pipeline(filename);
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
    pub fn reset(&self) {
        self.pipeline
            .remove_many([
                &self.filesrc,
                &self.decodebin,
                &self.binsink.clone().upcast(),
            ])
            .unwrap();
        gst::Element::unlink_many([&self.filesrc, &self.decodebin]);
        self.added.set(false);
    }

    fn run_pipeline(&self, filename: Option<&str>) {
        self.filesrc.set_property("location", filename);
        let sink: Element = self.binsink.clone().upcast();
        self.pipeline
            .add_many([&self.filesrc, &self.decodebin, &sink])
            .unwrap();
        gst::Element::link_many([&self.filesrc, &self.decodebin]).unwrap();

        let pipeline_weak = self.pipeline.downgrade();
        let sink_weak = sink.downgrade();
        let audiosink_weak = self.audioqueue.downgrade();
        self.added.set(true);

        self.decodebin.connect_pad_added(move |dbin, src_pad| {
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
            println!("new pad {:?}\n", src_pad);
            let pipeline = match pipeline_weak.upgrade() {
                Some(pipeline) => pipeline,
                None => return,
            };
            let sink = match sink_weak.upgrade() {
                Some(sink) => sink,
                None => return,
            };
            let audiosink = match audiosink_weak.upgrade() {
                Some(audiosink) => audiosink,
                None => return,
            };
            let insert_sink = |is_audio, is_video| -> Result<(), Error> {
                if is_audio {
                    let sink_pad = audiosink.static_pad("sink").expect("queue has no sinkpad");
                    if !src_pad.is_linked() {
                        src_pad.link(&sink_pad).expect("Unlink");
                    }
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
    pub fn build_pipeline(&self) {
        let element_arr = [&self.tee, &self.queue0, &self.videoconvert, &self.gtksink];
        self.binsink.add_many(element_arr).unwrap();

        gst::Element::link_many(element_arr).unwrap();
        self.binsink
            .add_pad(&gst::GhostPad::with_target(&self.tee.static_pad("sink").unwrap()).unwrap())
            .unwrap();

        let elements = &[
            &self.audioqueue,
            &self.audioconvert,
            &self.audioresample,
            &self.audiosink,
        ];
        self.pipeline.add_many(elements).unwrap();
        gst::Element::link_many(elements).unwrap();

        for e in elements {
            e.sync_state_with_parent().unwrap();
        }
    }
}
