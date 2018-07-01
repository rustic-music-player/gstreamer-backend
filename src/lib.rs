extern crate rustic_core as core;
extern crate gstreamer as gst;
#[macro_use]
extern crate failure;
#[macro_use]
extern crate log;
extern crate crossbeam_channel as channel;

use core::{PlayerBackend, PlayerState, PlayerEvent, Track};
use failure::{Error, err_msg};
use std::time::Duration;
use channel::{Sender, Receiver};
use gst::{prelude::*, MessageView, StateChangeReturn};
use std::sync::Arc;
use std::thread;
use std::any::Any;

#[derive(Debug)]
pub struct GstBackend {
    queue: Vec<Track>,
    current_index: usize,
    current_track: Option<Track>,
    current_volume: f32,
    state: PlayerState,
    blend_time: Duration,
    pipeline: gst::Pipeline,
    decoder: gst::Element,
    volume: gst::Element,
    sink: gst::Element,
    tx: Sender<PlayerEvent>,
    rx: Receiver<PlayerEvent>
}

impl GstBackend {
    pub fn new() -> Result<Arc<Box<PlayerBackend>>, Error> {
        gst::init()?;
        let pipeline = gst::Pipeline::new(None);
        let decoder = gst::ElementFactory::make("uridecodebin", None)
            .ok_or_else(|| err_msg("can't build uridecodebin"))?;
        let volume = gst::ElementFactory::make("volume", None)
            .ok_or_else(|| err_msg("can't build volume"))?;
        let sink = gst::ElementFactory::make("autoaudiosink", None)
            .ok_or_else(|| err_msg("can't build autoaudiosink"))?;
        let (tx, rx) = channel::unbounded();
        let backend = GstBackend {
            queue: vec![],
            current_index: 0,
            current_track: None,
            blend_time: Duration::default(),
            current_volume: 1.0,
            state: PlayerState::Stop,
            pipeline,
            decoder,
            volume,
            sink,
            tx,
            rx
        };

        backend.pipeline.add(&backend.decoder)?;
        backend.pipeline.add(&backend.volume)?;
        backend.pipeline.add(&backend.sink)?;

        backend.volume.link(&backend.sink)?;

        let sink_pad = backend.volume.get_static_pad("sink").ok_or_else(|| err_msg("missing sink pad on volume element"))?;
        backend.decoder.connect_pad_added(move |_el: &gst::Element, pad: &gst::Pad| {
            pad.link(&sink_pad);
        });

        let backend: Arc<Box<PlayerBackend>> = Arc::new(Box::new(backend));

        {
            let gst_backend = Arc::clone(&backend);
            let mut backend = Arc::clone(&backend);
            thread::spawn(move|| {
                let gst_backend: &GstBackend = match gst_backend.as_any().downcast_ref::<GstBackend>() {
                    Some(b) => b,
                    None => panic!("Not a GstBackend")
                };
                if let Some(bus) = gst_backend.pipeline.get_bus() {
                    let res: Result<(), Error> = match bus.pop() {
                        None => Ok(()),
                        Some(msg) => {
                            match msg.view() {
                                MessageView::Eos(..) => {
                                    println!("eos");
                                    let backend = Arc::get_mut(&mut backend).unwrap();
                                    backend.next()?;
                                    Ok(())
                                },
                                MessageView::Error(err) => {
                                    bail!(
                                        "Error from {}: {} ({:?})",
                                        msg.get_src().unwrap().get_path_string(),
                                        err.get_error(),
                                        err.get_debug()
                                    );
                                },
                                _ => Ok(()),
                            }
                        },
                    };
                }
                Ok(())
            });
        }

        Ok(backend)
    }

    fn set_track(&mut self, track: &Track) -> Result<(), Error> {
        debug!("Selecting {:?}", track);
        if let StateChangeReturn::Failure = self.pipeline.set_state(gst::State::Null) {
            bail!("can't stop pipeline")
        }
        self.decoder.set_property_from_str("uri", track.stream_url.as_str());

        let state = match self.state {
            PlayerState::Play => gst::State::Playing,
            PlayerState::Pause => gst::State::Paused,
            PlayerState::Stop => gst::State::Null
        };

        if let StateChangeReturn::Failure = self.pipeline.set_state(state) {
            bail!("can't restart pipeline")
        }
        Ok(())
    }
}

impl PlayerBackend for GstBackend {
    fn enqueue(&mut self, track: &Track) {
        self.queue.push(track.clone());
    }

    fn enqueue_multiple(&mut self, tracks: &[Track]) {
        self.queue.append(&mut tracks.to_vec());
    }

    fn play_next(&mut self, track: &Track) {
        self.queue.insert(self.current_index + 1, track.clone());
    }

    fn queue(&self) -> Vec<Track> {
        self.queue.clone()
    }

    fn clear_queue(&mut self) {
        self.queue.clear();
    }

    fn current(&self) -> Option<Track> {
        self.current_track.clone()
    }

    fn prev(&mut self) -> Result<Option<()>, Error> {
        if self.current_index == 0 {
            self.set_state(PlayerState::Stop)?;
            return Ok(None);
        }
        self.current_index -= 1;
        self.current_track = self.queue.get(self.current_index).cloned();
        if let Some(track) = self.current_track.clone() {
            self.set_track(&track)?;
            Ok(Some(()))
        }else {
            Ok(None)
        }
    }

    fn next(&mut self) -> Result<Option<()>, Error> {
        if self.current_index >= self.queue.len() {
            self.set_state(PlayerState::Stop)?;
            return Ok(None);
        }
        self.current_index += 1;
        self.current_track = self.queue.get(self.current_index).cloned();
        if let Some(track) = self.current_track.clone() {
            self.set_track(&track)?;
            Ok(Some(()))
        }else {
            Ok(None)
        }
    }

    fn set_state(&mut self, new_state: PlayerState) -> Result<(), Error> {
        let gst_state = match new_state {
            PlayerState::Play => gst::State::Playing,
            PlayerState::Pause => gst::State::Paused,
            PlayerState::Stop => gst::State::Null
        };
        if let StateChangeReturn::Failure = self.pipeline.set_state(gst_state) {
            bail!("can't play pipeline")
        }
        self.state = new_state;
        Ok(())
    }

    fn state(&self) -> PlayerState {
        self.state
    }

    fn set_volume(&mut self, volume: f32) -> Result<(), Error> {
        unimplemented!()
    }

    fn volume(&self) -> f32 {
        self.current_volume
    }

    fn set_blend_time(&mut self, duration: Duration) -> Result<(), Error> {
        unimplemented!()
    }

    fn blend_time(&self) -> Duration {
        self.blend_time
    }

    fn seek(&mut self, duration: Duration) -> Result<(), Error> {
        unimplemented!()
    }

    fn observe(&self) -> Receiver<PlayerEvent> {
        unimplemented!()
    }

    fn as_any(&self) -> &Any {
        self
    }
}