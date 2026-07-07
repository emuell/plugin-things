use std::any::TypeId;
use std::cell::RefCell;
use std::ffi::CStr;
use std::iter::zip;
use std::ptr::null_mut;
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use plinth_core::signals::ptr_signal::{PtrSignal, PtrSignalMut};
use plinth_core::signals::signal::SignalMut;
use vst3::Steinberg::Vst::ControllerNumbers_::{kAfterTouch, kCtrlProgramChange, kPitchBend};
use vst3::Steinberg::Vst::NoteExpressionTypeIDs_::{kBrightnessTypeID, kExpressionTypeID, kInvalidTypeID, kPanTypeID, kTuningTypeID, kVibratoTypeID, kVolumeTypeID};
use vst3::Steinberg::Vst::PhysicalUITypeIDs_::{kPUIPressure, kPUIXMovement, kPUIYMovement};
use vst3::Steinberg::Vst::{CtrlNumber, IMidiMapping, IMidiMappingTrait, INoteExpressionController, INoteExpressionControllerTrait, INoteExpressionPhysicalUIMapping, INoteExpressionPhysicalUIMappingTrait, NoteExpressionTypeID, NoteExpressionTypeInfo, NoteExpressionValue, PhysicalUIMapList};
use vst3::{ComPtr, ComRef};
use vst3::Steinberg::{int16, int32, kInvalidArgument, kNoInterface, kResultFalse, kResultOk, kResultTrue, tresult, uint32, FIDString, FUnknown, IBStream, IPlugView, IPluginBaseTrait, TBool, TUID};
use vst3::Steinberg::Vst::{kInfiniteTail, kNoParentUnitId, kNoProgramListId, kNoTail, BusDirection, BusDirections_, BusInfo, BusInfo_::BusFlags_, BusTypes_, CString, IAudioProcessor, IAudioProcessorTrait, IComponent, IComponentHandler, IComponentTrait, IEditController, IEditController2, IEditController2Trait, IEditControllerTrait, IHostApplication, IHostApplicationTrait, IProcessContextRequirements, IProcessContextRequirementsTrait, IProcessContextRequirements_, IUnitInfo, IUnitInfoTrait, IoMode, IoModes_, KnobMode, MediaType, MediaTypes_, ParamID, ParamValue, ParameterInfo_, ProcessData, ProcessSetup, ProgramListID, ProgramListInfo, RoutingInfo, SpeakerArr, SpeakerArrangement, String128, SymbolicSampleSizes_, TChar, UnitID, UnitInfo, ViewType::kEditor};
use widestring::U16CStr;

use crate::formats::PluginFormat;
use crate::host::HostInfo;
use crate::vst3::parameters::{parameter_change_to_event, MidiParameterIds};
use crate::{NoteExpressions, ParameterId, Parameters, ProcessMode, ProcessState, Processor, ProcessorConfig};
use crate::editor::NoEditor;
use crate::parameters::{group::{self, ParameterGroupRef}, has_duplicates, info::ParameterInfo};
use crate::string::{char16_to_string, copy_str_to_char16};
use crate::vst3::{event::{EventIterator, NoteIdMap}, parameters::ParameterChangeIterator};

use super::{plugin::Vst3Plugin, stream::Stream, view::View};

const ROOT_UNIT_NAME: &str  = "Root";
const ROOT_UNIT_ID: i32     = 0;
const FIRST_UNIT_ID: i32    = 1;

pub struct AudioThreadState<P: Vst3Plugin> {
    processor: parking_lot::Mutex<Option<P::Processor>>,
    aux_active: AtomicBool,
}

impl<P: Vst3Plugin> Default for AudioThreadState<P> {
    fn default() -> Self {
        Self {
            processor: Default::default(),
            aux_active: true.into(),
        }
    }
}

pub struct PluginComponent<P: Vst3Plugin> {
    plugin: Rc<RefCell<Option<P>>>,

    parameter_info: RefCell<Vec<ParameterInfo>>,
    parameter_groups: RefCell<Vec<ParameterGroupRef>>,
    midi_parameter_ids: RefCell<MidiParameterIds>,
    note_id_map: RefCell<NoteIdMap>,

    process_mode: RefCell<ProcessMode>,
    processing: AtomicBool,
    tail_length: AtomicU32,
    latency: AtomicU32,
    component_handler: Rc<RefCell<Option<ComPtr<IComponentHandler>>>>,

    audio_thread_state: AudioThreadState<P>,
}

impl<P: Vst3Plugin + 'static> PluginComponent<P> {
    pub fn new() -> Self {
        Self {
            plugin: Default::default(),

            parameter_info: Default::default(),
            parameter_groups: Default::default(),
            midi_parameter_ids: Default::default(),
            note_id_map: Default::default(),

            process_mode: ProcessMode::default().into(),
            processing: AtomicBool::new(false),
            tail_length: AtomicU32::new(0),
            latency: AtomicU32::new(0),

            component_handler: Default::default(),

            audio_thread_state: Default::default(),
        }
    }

    fn parameter_group_id(&self, parameter_info: &ParameterInfo) -> i32 {
        let parameter_path = parameter_info.path();
        if parameter_path.is_empty() {
            return ROOT_UNIT_ID;
        }

        let unit_index = self.parameter_groups.borrow().iter().position(|group| group.path == parameter_path).unwrap() as i32;
        FIRST_UNIT_ID + unit_index
    }
}

impl<P: Vst3Plugin> vst3::Class for PluginComponent<P> {
    type Interfaces = (IAudioProcessor, IComponent, IComponent, IEditController, IEditController2, IMidiMapping, IProcessContextRequirements, IUnitInfo, INoteExpressionController, INoteExpressionPhysicalUIMapping);
}

impl<P: Vst3Plugin> IPluginBaseTrait for PluginComponent<P> {
    unsafe fn initialize(&self, context: *mut FUnknown) -> tresult {
        tracing::trace!("IPluginBase::initialize");

        if self.plugin.borrow().is_some() {
            return kResultOk;
        }

        // Get plugin name if available
        let mut host_name = None;

        if let Some(context) = unsafe { ComRef::from_raw(context) } && let Some(host_application) = context.cast::<IHostApplication>() {
            let mut name = [0; 128];

            if unsafe { host_application.getName(&mut name) == kResultOk } && let Some(name) = char16_to_string(&name) {
                host_name = Some(name);
            }
        }

        // Create plugin and find parameter info
        let host_info = HostInfo {
            name: host_name,
            format: PluginFormat::Vst3,
        };

        let mut plugin = P::new(host_info);
        assert!(plugin.with_parameters(|parameters| !has_duplicates(parameters.ids())));

        plugin.init();

        let mut parameter_infos = self.parameter_info.borrow_mut();

        // Create units based on parameter groups
        // Also verify parameters
        *self.parameter_groups.borrow_mut() = plugin.with_parameters(|parameters| {
            assert!(
                parameters.ids().iter()
                    .copied()
                    .filter(|&id| parameters.get(id).unwrap().info().is_bypass())
                    .count() <= 1,
                "You can only define one bypass parameter"
            );

            for &id in parameters.ids() {
                let info = parameters.get(id).unwrap().info();
                parameter_infos.push(info.clone());
            }

            group::from_parameters(parameters)
        });

        // Allocate hidden reserved VST3 parameters for each MIDI message type the plugin requires via its MIDI_CAPABILITIES.
        plugin.with_parameters(|parameters| {
            let user_ids = parameters.ids();
            let mut next_id: ParameterId = 1;

            // Allocate one 16-channel block, pushing hidden ParameterInfos.
            let mut alloc_block = |infos: &mut Vec<ParameterInfo>, name_fn: &dyn Fn(usize) -> String| -> [ParameterId; 16] {
                let mut block = [0u32; 16];
                for (channel, slot) in block.iter_mut().enumerate() {
                    while user_ids.contains(&next_id) {
                        next_id += 1;
                    }
                    infos.push(ParameterInfo::new(next_id, name_fn(channel)).hidden());
                    *slot = next_id;
                    next_id += 1;
                }
                block
            };

            let mut midi_ids = MidiParameterIds::default();

            if P::MIDI_CAPABILITIES.midi_pitch_bend() {
                midi_ids.pitch_bend = Some(alloc_block(&mut parameter_infos, &|channel| format!("MIDI Channel {} Pitch Bend", channel + 1)));
            }

            if P::MIDI_CAPABILITIES.midi_channel_pressure() {
                midi_ids.channel_pressure = Some(alloc_block(&mut parameter_infos, &|channel| format!("MIDI Channel {} Channel Pressure", channel + 1)));
            }

            if P::MIDI_CAPABILITIES.midi_program_change() {
                midi_ids.program_change = Some(alloc_block(&mut parameter_infos, &|channel| format!("MIDI Channel {} Program Change", channel + 1)));
            }

            for cc in P::MIDI_CAPABILITIES.enabled_midi_control_changes() {
                let block = alloc_block(&mut parameter_infos, &|channel| format!("MIDI Channel {} CC {}", channel + 1, cc));
                midi_ids.cc.insert(cc, block);
            }

            *self.midi_parameter_ids.borrow_mut() = midi_ids;
        });

        *self.plugin.borrow_mut() = Some(plugin);

        kResultOk
    }

    unsafe fn terminate(&self) -> tresult {
        tracing::trace!("IPluginBase::terminate");

        *self.plugin.borrow_mut() = None;
        self.parameter_info.borrow_mut().clear();
        self.parameter_groups.borrow_mut().clear();

        kResultOk
    }
}

impl<P: Vst3Plugin> IAudioProcessorTrait for PluginComponent<P> {
    unsafe fn setBusArrangements(&self, inputs: *mut SpeakerArrangement, num_ins: int32, outputs: *mut SpeakerArrangement, num_outs: int32) -> tresult {
        tracing::trace!("IAudioProcessor::setBusArrangements");

        if inputs.is_null() || outputs.is_null() {
            return kInvalidArgument;
        }

        let expected_inputs = if P::HAS_AUX_INPUT { 2 } else { 1 };
        if num_ins != expected_inputs {
            return kResultFalse;
        }

        if num_outs != 1 {
            return kResultFalse;
        }

        let inputs = unsafe { std::slice::from_raw_parts(inputs, num_ins as _) };
        if inputs[0] != SpeakerArr::kStereo {
            return kResultFalse;
        }
        if P::HAS_AUX_INPUT && inputs[1] != SpeakerArr::kStereo {
            return kResultFalse;
        }

        let outputs = unsafe { std::slice::from_raw_parts(outputs, num_outs as _) };
        if outputs[0] != SpeakerArr::kStereo {
            return kResultFalse;
        }

        kResultOk
    }

    unsafe fn getBusArrangement(&self, _dir: BusDirection, _index: int32, arr: *mut SpeakerArrangement) -> tresult {
        tracing::trace!("IAudioProcessor::getBusArrangements");

        // Only support stereo
        unsafe { *arr = SpeakerArr::kStereo; }
        kResultOk
    }

    unsafe fn canProcessSampleSize(&self, symbolic_sample_size: int32) -> tresult {
        tracing::trace!("IAudioProcessor::canProcessSampleSize");

        if symbolic_sample_size == SymbolicSampleSizes_::kSample32 as int32 {
            kResultOk
        } else {
            kResultFalse
        }
    }

    unsafe fn getLatencySamples(&self) -> uint32 {
        tracing::trace!("IAudioProcessor::getLatencySamples");
        self.latency.load(Ordering::Acquire)
    }

    unsafe fn setupProcessing(&self, setup: *mut ProcessSetup) -> tresult {
        tracing::trace!("IAudioProcessor::setupProcessing");

        let setup = unsafe { &*setup };

        let processor_config = ProcessorConfig {
            sample_rate: setup.sampleRate,
            min_block_size: 0,
            max_block_size: setup.maxSamplesPerBlock as _,
            process_mode: *self.process_mode.borrow(),
        };

        let plugin = self.plugin.borrow();
        let Some(plugin) = plugin.as_ref() else {
            return kResultFalse;
        };

        let mut processor = self.audio_thread_state.processor.lock();
        *processor = Some(plugin.create_processor(processor_config));

        // Cache latency since it's not allowed to change during processing
        self.latency.store(plugin.latency(), Ordering::Release);

        kResultOk
    }

    unsafe fn setProcessing(&self, state: TBool) -> tresult {
        tracing::trace!("IAudioProcessor::setProcessing: {state}");

        let processing = state != 0;
        self.processing.store(processing, Ordering::Release);

        let mut processor = self.audio_thread_state.processor.lock();
        if let Some(processor) = processor.as_mut() && !processing {
            processor.reset();
        }

        if let Ok(mut note_id_map) = self.note_id_map.try_borrow_mut() && !processing {
            note_id_map.reset();
        }
        
        kResultOk
    }

    // Called from the audio thread
    unsafe fn process(&self, data: *mut ProcessData) -> tresult {
        let data = unsafe { &mut *data };

        let midi_ids = self.midi_parameter_ids.borrow();
        let parameter_change_iterator = ParameterChangeIterator::new(data.inputParameterChanges, &midi_ids);
        let mut note_id_map = self.note_id_map.borrow_mut();
        let event_iterator = EventIterator::new(data.inputEvents, &mut note_id_map, P::NOTE_EXPRESSIONS);
        let all_events = event_iterator.chain(parameter_change_iterator);
        let is_data_dump = data.inputs.is_null() || data.outputs.is_null() || data.numInputs == 0 || data.numSamples == 0;

        // On some platforms, this cast is needed
        #[allow(clippy::unnecessary_cast)]
        if !is_data_dump && data.symbolicSampleSize != SymbolicSampleSizes_::kSample32 as i32 {
            return kResultFalse;
        }

        // Prepare inputs & outputs
        let (main_input, main_output, aux_input) = if is_data_dump {
            (None, None, None)
        } else {
            let inputs = unsafe { std::slice::from_raw_parts(data.inputs, data.numInputs as _) };
            let outputs = unsafe { std::slice::from_raw_parts(data.outputs, data.numOutputs as _) };
            let main_input = inputs[0];
            let main_output = outputs[0];
            assert_eq!(main_input.numChannels, main_output.numChannels);

            let aux_input = if P::HAS_AUX_INPUT && self.audio_thread_state.aux_active.load(Ordering::Acquire) {
                assert_eq!(data.numInputs, 2);
                let aux_input = inputs[1];
                Some(unsafe { PtrSignal::from_pointers(aux_input.numChannels as usize, data.numSamples as usize, aux_input.__field0.channelBuffers32 as _) })
            } else {
                None
            };

            let main_input = unsafe { PtrSignal::from_pointers(main_input.numChannels as usize, data.numSamples as usize, main_input.__field0.channelBuffers32 as _) };
            let main_output = unsafe { PtrSignalMut::from_pointers(main_output.numChannels as usize, data.numSamples as usize, main_output.__field0.channelBuffers32) };

            (Some(main_input), Some(main_output), aux_input)
        };

        // Real-time safety: parking_lot Mutex is guaranteed to not do syscalls when uncontented
        // contention can only occur if we're setting up or tearing down the processor while process is called
        // In that case, we will simply output silence
        let Some(mut processor) = self.audio_thread_state.processor.try_lock() else {
            if let Some(mut main_output) = main_output {
                main_output.fill(0.0);
            }

            return kResultOk;
        };

        let Some(processor) = processor.as_mut() else {
            return kResultFalse;
        };

        if is_data_dump {
            processor.process_events(all_events);
            return kResultOk;
        }

        let main_input = main_input.unwrap();
        let mut main_output = main_output.unwrap();

        // If processing out-of-place, copy input to output
        if zip(main_input.pointers().iter(), main_output.pointers().iter())
            .any(|(&input_ptr, &output_ptr)| input_ptr != unsafe { &*output_ptr })
        {
            main_output.copy_from_signal(&main_input);
        }

        let transport = if data.processContext.is_null() {
            None
        } else {
            Some(unsafe { &*data.processContext }.into())
        };

        let process_state = processor.process(&mut main_output, aux_input.as_ref(), transport, all_events);

        let tail_length = match process_state {
            ProcessState::Error => {
                tracing::error!("Processing error!");
                return kResultFalse;
            },

            ProcessState::Normal | ProcessState::Tail(0) => kNoTail,
            ProcessState::Tail(tail) => tail as _,
            ProcessState::KeepAlive => kInfiniteTail,
        };

        self.tail_length.store(tail_length, Ordering::Release);

        kResultOk
    }

    unsafe fn getTailSamples(&self) -> uint32 {
        self.tail_length.load(Ordering::Acquire)
    }
}

impl<P: Vst3Plugin> IComponentTrait for PluginComponent<P> {
    unsafe fn getControllerClassId(&self, _class_id: *mut TUID) -> tresult {
        tracing::trace!("IComponent::getControllerClassId");
        kNoInterface
    }

    unsafe fn setIoMode(&self, mode: IoMode) -> tresult {
        tracing::trace!("IComponent::setIoMode");

        let mode = match mode as _ {
            IoModes_::kSimple | IoModes_::kAdvanced => ProcessMode::Realtime,
            IoModes_::kOfflineProcessing => ProcessMode::Offline,
            _ => {
                return kInvalidArgument;
            }
        };

        *self.process_mode.borrow_mut() = mode;

        kResultOk
    }

    unsafe fn getBusCount(&self, media_type: MediaType, dir: BusDirection) -> int32 {
        tracing::trace!("IComponent::getBusCount");

        // On some platforms, these casts are needed
        #[allow(clippy::unnecessary_cast)]
        if P::HAS_AUX_INPUT && media_type == MediaTypes_::kAudio as i32 && dir == BusDirections_::kInput as i32 {
            2
        } else {
            1
        }
    }

    unsafe fn getBusInfo(&self, media_type: MediaType, dir: BusDirection, index: int32, bus: *mut BusInfo) -> tresult {
        tracing::trace!("IComponent::getBusInfo");

        if index >= unsafe { self.getBusCount(media_type, dir) } {
            return kInvalidArgument;
        }

        let bus = unsafe { &mut *bus };
        bus.mediaType = media_type;
        bus.direction = dir;
        bus.flags = BusFlags_::kDefaultActive as _;

        if index == 0 {
            copy_str_to_char16("Main", &mut bus.name);
            bus.busType = BusTypes_::kMain as _;
        } else {
            copy_str_to_char16("Aux", &mut bus.name);
            bus.busType = BusTypes_::kAux as _;
        }

        bus.channelCount = match media_type as _ {
            MediaTypes_::kAudio => 2,
            MediaTypes_::kEvent => 16,
            _ => { return kInvalidArgument }
        };

        kResultOk
    }

    unsafe fn getRoutingInfo(&self, in_info: *mut RoutingInfo, out_info: *mut RoutingInfo) -> tresult {
        tracing::trace!("IComponent::getRoutingInfo");

        let in_info = unsafe { &*in_info };
        let out_info = unsafe { &mut *out_info };

        out_info.mediaType = in_info.mediaType;
        out_info.busIndex = in_info.busIndex;
        out_info.channel = in_info.channel;

        kResultOk
    }

    unsafe fn activateBus(&self, media_type: MediaType, dir: BusDirection, index: int32, state: TBool) -> tresult {
        tracing::trace!("IComponent::activateBus");

        // On some platforms, these casts are needed
        #[allow(clippy::unnecessary_cast)]
        if P::HAS_AUX_INPUT && media_type == MediaTypes_::kAudio as i32 && dir == BusDirections_::kInput as i32 && index == 1 {
            self.audio_thread_state.aux_active.store(state != 0, Ordering::Release);
        }

        // TODO: Support disabling other buses
        kResultOk
    }

    unsafe fn setActive(&self, _state: TBool) -> tresult {
        tracing::trace!("IComponent::setActive: {_state}");
        kResultOk
    }

    unsafe fn setState(&self, state: *mut IBStream) -> tresult {
        tracing::trace!("IComponent::setState");

        let mut plugin = self.plugin.borrow_mut();
        let Some(plugin) = plugin.as_mut() else {
            return kResultFalse;
        };

        let Some(mut stream) = Stream::new(state) else {
            return kResultFalse;
        };

        match plugin.load_state(&mut stream) {
            Ok(_) => kResultOk,
            Err(_) => kResultFalse, // TODO: Extract actual error code
        }
    }

    unsafe fn getState(&self, state: *mut IBStream) -> tresult {
        tracing::trace!("IComponent::getState");

        let plugin = self.plugin.borrow();
        let Some(plugin) = plugin.as_ref() else {
            return kResultFalse;
        };
        let Some(mut stream) = Stream::new(state) else {
            return kResultFalse;
        };

        match plugin.save_state(&mut stream) {
            Ok(_) => kResultOk,
            Err(_) => kResultFalse, // TODO: Extract actual error code
        }
    }
}

impl<P: Vst3Plugin + 'static> IEditControllerTrait for PluginComponent<P> {
    unsafe fn setComponentState(&self, _state: *mut IBStream) -> tresult {
        tracing::trace!("IEditController::setComponentState");
        kResultOk
    }

    unsafe fn setState(&self, _state: *mut IBStream) -> tresult {
        tracing::trace!("IEditController::setState");
        kResultOk
    }

    unsafe fn getState(&self, _state: *mut IBStream) -> tresult {
        tracing::trace!("IEditController::getState");
        kResultOk
    }

    unsafe fn getParameterCount(&self) -> int32 {
        tracing::trace!("IEditController::getParameterCount");
        self.parameter_info.borrow().len() as _
    }

    unsafe fn getParameterInfo(&self, param_index: int32, info: *mut vst3::Steinberg::Vst::ParameterInfo) -> tresult {
        tracing::trace!("IEditController::getParameterInfo");

        if param_index < 0 {
            return kInvalidArgument;
        }

        let parameter_info = self.parameter_info.borrow();
        let Some(parameter_info) = parameter_info.get(param_index as usize) else {
            return kInvalidArgument;
        };

        let vst3_info = unsafe { &mut *info };

        vst3_info.id = parameter_info.id();
        copy_str_to_char16(parameter_info.name(), &mut vst3_info.title);
        // TODO: info.shortTitle
        vst3_info.stepCount = parameter_info.steps() as _;
        vst3_info.defaultNormalizedValue = parameter_info.default_normalized_value();
        vst3_info.unitId = self.parameter_group_id(parameter_info);

        #[allow(clippy::unnecessary_cast)]
        if parameter_info.is_bypass() {
            vst3_info.flags = ParameterInfo_::ParameterFlags_::kIsBypass as i32;
            vst3_info.flags |= ParameterInfo_::ParameterFlags_::kCanAutomate as i32;
        } else if !parameter_info.visible() {
            vst3_info.flags = ParameterInfo_::ParameterFlags_::kIsHidden as i32;
        } else {
            vst3_info.flags = ParameterInfo_::ParameterFlags_::kCanAutomate as i32;
        }

        kResultOk
    }

    unsafe fn getParamStringByValue(&self, id: ParamID, value_normalized: ParamValue, string: *mut String128) -> tresult {
        tracing::trace!("IEditController::getParamStringByValue");

        let plugin = self.plugin.borrow();
        let Some(plugin) = plugin.as_ref() else {
            return kResultFalse;
        };

        plugin.with_parameters(|parameters| {
            let Some(parameter) = parameters.get(id) else {
                return kInvalidArgument;
            };

            let formatted = parameter.normalized_to_string(value_normalized);
            copy_str_to_char16(&formatted, unsafe { &mut *string });

            kResultOk
        })
    }

    unsafe fn getParamValueByString(&self, id: ParamID, string: *mut TChar, value_normalized: *mut ParamValue) -> tresult {
        tracing::trace!("IEditController::getParamValueByString");

        if string.is_null() {
            return kInvalidArgument;
        }

        let plugin = self.plugin.borrow();
        let Some(plugin) = plugin.as_ref() else {
            return kResultFalse;
        };

        let string = unsafe { U16CStr::from_ptr_str(string as _) };
        let Ok(string) = string.to_string() else {
            return kInvalidArgument;
        };

        plugin.with_parameters(|parameters| {
            let Some(parameter) = parameters.get(id) else {
                return kInvalidArgument;
            };

            let Some(value) = parameter.string_to_normalized(&string) else {
                return kInvalidArgument;
            };

            unsafe { *value_normalized = value };

            kResultOk
        })
    }

    unsafe fn normalizedParamToPlain(&self, _id: ParamID, value_normalized: ParamValue) -> ParamValue {
        value_normalized
    }

    unsafe fn plainParamToNormalized(&self, _id: ParamID, plain_value: ParamValue) -> ParamValue {
        plain_value
    }

    unsafe fn getParamNormalized(&self, id: ParamID) -> ParamValue {
        tracing::trace!("IEditController::getParamNormalized");

        let plugin = self.plugin.borrow();
        let Some(plugin) = plugin.as_ref() else {
            return 0.0;
        };

        plugin.with_parameters(|parameters| {
            let Some(parameter) = parameters.get(id) else {
                return 0.0;
            };

            parameter.normalized_value()
        })
    }

    unsafe fn setParamNormalized(&self, id: ParamID, value: ParamValue) -> tresult {
        tracing::trace!("IEditController::setParamNormalized");

        let mut plugin = self.plugin.borrow_mut();
        let Some(plugin) = plugin.as_mut() else {
            return kResultFalse;
        };

        let event = parameter_change_to_event(id, value, 0, &self.midi_parameter_ids.borrow());
        plugin.process_event(&event);

        kResultOk
    }

    unsafe fn setComponentHandler(&self, handler: *mut IComponentHandler) -> tresult {
        tracing::trace!("IEditController::setComponentHandler: {:x}", handler as usize);

        if handler.is_null() {
            *self.component_handler.borrow_mut() = None;
        } else {
            let Some(handler) = (unsafe { ComRef::from_raw(handler) }) else {
                return kInvalidArgument;
            };

            *self.component_handler.borrow_mut() = Some(handler.to_com_ptr());
        }

        kResultOk
    }

    unsafe fn createView(&self, name: FIDString) -> *mut IPlugView {
        tracing::trace!("IEditController::createView");

        if name.is_null() {
            return null_mut();
        }

        if unsafe { CStr::from_ptr(name) != CStr::from_ptr(kEditor) } {
            return null_mut();
        }

        if TypeId::of::<P::Editor>() == TypeId::of::<NoEditor>() {
            return null_mut();
        }

        let view = View::<P>::new(
            self.plugin.clone(),
            self.component_handler.clone(),
        );

        view.to_com_ptr::<IPlugView>().unwrap().into_raw()
    }
}

impl<P: Vst3Plugin> IEditController2Trait for PluginComponent<P> {
    unsafe fn setKnobMode(&self, _mode: KnobMode) -> tresult {
        tracing::trace!("IEditController2::setKnobMode");
        kResultFalse
    }

    unsafe fn openHelp(&self, _only_check: TBool) -> tresult {
        tracing::trace!("IEditController2::openHelp");
        kResultFalse
    }

    unsafe fn openAboutBox(&self, _only_check: TBool) -> tresult {
        tracing::trace!("IEditController2::openAboutBox");
        kResultFalse
    }
}

impl<P: Vst3Plugin> IMidiMappingTrait for PluginComponent<P> {
    unsafe fn getMidiControllerAssignment(
        &self,
        bus_index: int32,
        channel: int16,
        midi_controller_number: CtrlNumber,
        id: *mut ParamID) -> tresult
    {
        if bus_index != 0 {
            return kResultFalse;
        }
        if !(0..16).contains(&channel) {
            return kInvalidArgument;
        }

        let midi_ids = self.midi_parameter_ids.borrow();
        let channel = channel as usize;

        if midi_controller_number == kPitchBend as i16 {
            if let Some(pb_ids) = &midi_ids.pitch_bend {
                unsafe { *id = pb_ids[channel] as _ };
                return kResultTrue;
            }
        } else if midi_controller_number == kAfterTouch as i16 {
            if let Some(cp_ids) = &midi_ids.channel_pressure {
                unsafe { *id = cp_ids[channel] as _ };
                return kResultTrue;
            }
        } else if midi_controller_number == kCtrlProgramChange as i16 {
            if let Some(pc_ids) = &midi_ids.program_change {
                unsafe { *id = pc_ids[channel] as _ };
                return kResultTrue;
            }
        } else if (0..128).contains(&midi_controller_number) {
            let cc = midi_controller_number as u8;
            if let Some(cc_ids) = midi_ids.cc.get(&cc) {
                unsafe { *id = cc_ids[channel] as _ };
                return kResultTrue;
            }
        }

        kResultFalse
    }
}

impl<P: Vst3Plugin> IProcessContextRequirementsTrait for PluginComponent<P> {
    unsafe fn getProcessContextRequirements(&self) -> uint32 {
        tracing::trace!("IProcessContextRequirements::getProcessContextRequirements");
        IProcessContextRequirements_::Flags_::kNeedContinousTimeSamples as uint32 |
        IProcessContextRequirements_::Flags_::kNeedProjectTimeMusic as uint32 |
        IProcessContextRequirements_::Flags_::kNeedBarPositionMusic as uint32 |
        IProcessContextRequirements_::Flags_::kNeedCycleMusic as uint32 |
        IProcessContextRequirements_::Flags_::kNeedTempo as uint32 |
        IProcessContextRequirements_::Flags_::kNeedTimeSignature as uint32 |
        IProcessContextRequirements_::Flags_::kNeedTransportState as uint32
    }
}

impl<P: Vst3Plugin> IUnitInfoTrait for PluginComponent<P> {
    unsafe fn getUnitCount(&self) -> int32 {
        tracing::trace!("IUnitInfo::getUnitCount");
        let parameter_groups = self.parameter_groups.borrow();
        parameter_groups.len() as int32 + 1 // +1 for the root unit
    }

    unsafe fn getUnitInfo(&self, unit_index: int32, info: *mut UnitInfo) -> tresult {
        tracing::trace!("IUnitInfo::getUnitInfo");

        let parameter_groups = self.parameter_groups.borrow();
        let unit_count = parameter_groups.len() + 1; // +1 for the root unit

        if unit_index < 0 {
            return kInvalidArgument;
        }
        if unit_index as usize >= unit_count {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        info.id = unit_index;
        info.programListId = kNoProgramListId;
        info.parentUnitId = kNoParentUnitId;

        // Special case root unit
        if unit_index == ROOT_UNIT_ID {
            copy_str_to_char16(ROOT_UNIT_NAME, &mut info.name);
        } else {
            let unit_index = unit_index - FIRST_UNIT_ID;
            let group = &parameter_groups[unit_index as usize];
            copy_str_to_char16(&group.name, &mut info.name);

            if let Some(parent) = &group.parent {
                info.parentUnitId = FIRST_UNIT_ID + parameter_groups.iter().position(|group| group == parent).unwrap() as i32;
            } else {
                info.parentUnitId = ROOT_UNIT_ID;
            }
        }

        kResultOk
    }

    unsafe fn getProgramListCount(&self) -> int32 {
        tracing::trace!("IUnitInfo::getProgramListCount");
        0
    }

    unsafe fn getProgramListInfo(&self, _list_index: int32, _info: *mut ProgramListInfo) -> tresult {
        tracing::trace!("IUnitInfo::getProgramListInfo");
        kInvalidArgument
    }

    unsafe fn getProgramName(&self, _list_id: ProgramListID, _program_index: int32, _name: *mut String128) -> tresult {
        tracing::trace!("IUnitInfo::getProgramName");
        kInvalidArgument
    }

    unsafe fn getProgramInfo(&self, _list_id: ProgramListID, _program_index: int32, _attribute_id: CString, _attribute_value: *mut String128) -> tresult {
        tracing::trace!("IUnitInfo::getProgramInfo");
        kInvalidArgument
    }

    unsafe fn hasProgramPitchNames(&self, _list_id: ProgramListID, _program_index: int32) -> tresult {
        tracing::trace!("IUnitInfo::hasProgramPitchNames");
        kInvalidArgument
    }

    unsafe fn getProgramPitchName(&self, _list_id: ProgramListID, _program_index: int32, _midi_pitch: int16, _name: *mut String128) -> tresult {
        tracing::trace!("IUnitInfo::getProgramPitchName");
        kInvalidArgument
    }

    unsafe fn getSelectedUnit(&self) -> UnitID {
        tracing::trace!("IUnitInfo::getSelectedUnit");
        0
    }

    unsafe fn selectUnit(&self, _unit_id: UnitID) -> tresult {
        tracing::trace!("IUnitInfo::selectUnit");
        kInvalidArgument
    }

    unsafe fn getUnitByBus(&self, _media_type: MediaType, _dir: BusDirection, _bus_index: int32, _channel: int32, _unit_id: *mut UnitID) -> tresult {
        tracing::trace!("IUnitInfo::getUnitByBus");
        kInvalidArgument
    }

    unsafe fn setUnitProgramData(&self, _list_or_unit_id: int32, _program_index: int32, _data: *mut IBStream) -> tresult {
        tracing::trace!("IUnitInfo::setUnitProgramData");
        kInvalidArgument
    }
}

// Standard VST3 note-expression types. Only the enabled subset (per `NoteExpressions` config) is exposed to the host.
struct NoteExpressionDescriptor {
    type_id: NoteExpressionTypeID,
    title: &'static str,
    short_title: &'static str,
    units: &'static str,
    default_value: NoteExpressionValue,
    enabled: fn(&NoteExpressions) -> bool,
}

const NOTE_EXPRESSION_DESCRIPTORS: [NoteExpressionDescriptor; 6] = [
    NoteExpressionDescriptor {
        type_id: kVolumeTypeID,
        title: "Volume",
        short_title: "Volume",
        units: "",
        default_value: 1.0,
        enabled: NoteExpressions::volume,
    },
    NoteExpressionDescriptor {
        type_id: kPanTypeID,
        title: "Panning",
        short_title: "Panning",
        units: "",
        default_value: 0.5, // center
        enabled: NoteExpressions::pan,
    },
    NoteExpressionDescriptor {
        type_id: kTuningTypeID,
        title: "Tuning",
        short_title: "Tuning",
        units: "semitones",
        default_value: 0.5, // center
        enabled: NoteExpressions::tuning,
    },
    NoteExpressionDescriptor {
        type_id: kVibratoTypeID,
        title: "Vibrato",
        short_title: "Vibrato",
        units: "",
        default_value: 0.0,
        enabled: NoteExpressions::vibrato,
    },
    NoteExpressionDescriptor {
        type_id: kExpressionTypeID,
        title: "Expression",
        short_title: "Expression",
        units: "",
        default_value: 0.0,
        enabled: NoteExpressions::expression,
    },
    NoteExpressionDescriptor {
        type_id: kBrightnessTypeID,
        title: "Brightness",
        short_title: "Brightness",
        units: "",
        default_value: 0.0,
        enabled: NoteExpressions::brightness,
    },
];

fn fill_note_expression_info(note_expressions: NoteExpressions, index: i32, info: &mut NoteExpressionTypeInfo) -> bool {
    let Ok(index) = usize::try_from(index) else {
        return false;
    };
    let Some(descriptor) = NOTE_EXPRESSION_DESCRIPTORS.iter().filter(|descriptor| (descriptor.enabled)(&note_expressions)).nth(index) else {
        return false;
    };

    info.typeId = descriptor.type_id;
    info.unitId = -1;
    info.associatedParameterId = u32::MAX;
    info.flags = 0;
    info.valueDesc.minimum = 0.0;
    info.valueDesc.maximum = 1.0;
    info.valueDesc.stepCount = 0;
    info.valueDesc.defaultValue = descriptor.default_value;
    copy_str_to_char16(descriptor.title, &mut info.title);
    copy_str_to_char16(descriptor.short_title, &mut info.shortTitle);
    copy_str_to_char16(descriptor.units, &mut info.units);
    true
}

impl<P: Vst3Plugin + 'static> INoteExpressionControllerTrait for PluginComponent<P> {
    unsafe fn getNoteExpressionCount(&self, bus_index: int32, channel: int16) -> int32 {
        tracing::trace!("INoteExpressionController::getNoteExpressionCount");
        if bus_index == 0 && (0..16).contains(&channel) {
            // NB: Pressure is excluded here. It is delivered as `kPolyPressureEvent`.
            P::NOTE_EXPRESSIONS.volume() as i32
                + P::NOTE_EXPRESSIONS.pan() as i32
                + P::NOTE_EXPRESSIONS.tuning() as i32
                + P::NOTE_EXPRESSIONS.vibrato() as i32
                + P::NOTE_EXPRESSIONS.expression() as i32
                + P::NOTE_EXPRESSIONS.brightness() as i32
        } else {
            0
        }
    }

    unsafe fn getNoteExpressionInfo(&self, bus_index: int32, channel: int16, note_expression_index: int32, info: *mut NoteExpressionTypeInfo) -> tresult {
        tracing::trace!("INoteExpressionController::getNoteExpressionInfo");

        if bus_index != 0 || !(0..16).contains(&channel) || info.is_null() {
            return kInvalidArgument;
        }

        let info = unsafe { &mut *info };
        if fill_note_expression_info(P::NOTE_EXPRESSIONS, note_expression_index, info) {
            kResultOk
        } else {
            kInvalidArgument
        }
    }

    unsafe fn getNoteExpressionStringByValue(&self, bus_index: int32, channel: int16, id: NoteExpressionTypeID, value_normalized: NoteExpressionValue, string: *mut String128) -> tresult {
        tracing::trace!("INoteExpressionController::getNoteExpressionStringByValue");

        if P::NOTE_EXPRESSIONS.is_empty() {
            return kInvalidArgument;
        }
        if bus_index != 0 || !(0..16).contains(&channel) || string.is_null() {
            return kInvalidArgument;
        }

        let s = unsafe { &mut *string };

        #[allow(non_upper_case_globals)]
        let formatted = match id {
            kVolumeTypeID => format!("{:.2}", value_normalized),
            kPanTypeID => {
                let pan = value_normalized * 200.0 - 100.0;
                format!("{:.1} %", pan)
            }
            kTuningTypeID => {
                let semitones = value_normalized * 240.0 - 120.0;
                format!("{:+.2} st", semitones)
            }
            kVibratoTypeID | kExpressionTypeID | kBrightnessTypeID => {
                format!("{:.2}", value_normalized)
            }
            _ => return kInvalidArgument,
        };
        copy_str_to_char16(&formatted, s);
        kResultOk
    }

    unsafe fn getNoteExpressionValueByString(&self, bus_index: int32, channel: int16, id: NoteExpressionTypeID, string: *const TChar, value_normalized: *mut NoteExpressionValue) -> tresult {
        tracing::trace!("INoteExpressionController::getNoteExpressionValueByString");

        if P::NOTE_EXPRESSIONS.is_empty() {
            return kInvalidArgument;
        }
        if bus_index != 0 || !(0..16).contains(&channel) || string.is_null() || value_normalized.is_null() {
            return kInvalidArgument;
        }

        let s = unsafe { U16CStr::from_ptr_str(string as _) };
        let Ok(s) = s.to_string() else {
            return kInvalidArgument;
        };
        // Strip everything except digits to get a plain number string
        let digits: String = s.chars().filter(|c| c.is_ascii_digit() || matches!(c, '.' | '-' | '+')).collect();

        #[allow(non_upper_case_globals)]
        match id {
            kTuningTypeID => {
                if let Ok(semitones) = digits.parse::<f64>() {
                    let value = (semitones + 120.0) / 240.0;
                    unsafe { *value_normalized = value.clamp(0.0, 1.0) };
                    kResultOk
                } else {
                    kResultFalse
                }
            }
            kPanTypeID => {
                if let Ok(pan_pct) = digits.parse::<f64>() {
                    // Input may be a % in -100..100
                    let value = (pan_pct + 100.0) / 200.0;
                    unsafe { *value_normalized = value.clamp(0.0, 1.0) };
                    kResultOk
                } else {
                    kResultFalse
                }
            }
            kVolumeTypeID | kVibratoTypeID | kExpressionTypeID | kBrightnessTypeID => {
                if let Ok(value) = digits.parse::<f64>() {
                    unsafe { *value_normalized = value.clamp(0.0, 1.0) };
                    kResultOk
                } else {
                    kResultFalse
                }
            }
            _ => kInvalidArgument,
        }
    }
}

impl<P: Vst3Plugin> INoteExpressionPhysicalUIMappingTrait for PluginComponent<P> {
    unsafe fn getPhysicalUIMapping(&self, bus_index: int32, channel: int16, list: *mut PhysicalUIMapList) -> tresult {
        tracing::trace!("INoteExpressionPhysicalUIMapping::getPhysicalUIMapping");

        if bus_index != 0 || !(0..16).contains(&channel) || list.is_null() {
            return kInvalidArgument;
        }

        let list = unsafe { &mut *list };

        for i in 0..list.count as usize {
            let entry = unsafe { &mut *list.map.add(i) };
            #[allow(non_upper_case_globals)]
            let ne_type = if entry.physicalUITypeID == kPUIXMovement as u32 {
                // Horizontal (slide left/right) -> per-note pitch / tuning
                if P::NOTE_EXPRESSIONS.tuning() {
                    kTuningTypeID
                } else {
                    kInvalidTypeID
                }
            } else if entry.physicalUITypeID == kPUIYMovement as u32 {
                // Vertical (slide up/down) -> brightness / timbre
                if P::NOTE_EXPRESSIONS.brightness() {
                    kBrightnessTypeID
                } else {
                    kInvalidTypeID
                }
            } else if entry.physicalUITypeID == kPUIPressure as u32 {
                // Pressure (Z-axis) -> delivered as kPolyPressureEvent, not note expression
                kInvalidTypeID
            } else {
                kInvalidTypeID
            };
            entry.noteExpressionTypeID = ne_type;
        }

        kResultOk
    }
}
