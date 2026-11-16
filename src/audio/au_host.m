// AudioBoard's Audio Unit host: a small C interface over AUAudioUnit, for
// src/audio/au.rs. One path serves Audio Unit v3 app extensions (loaded out
// of process, as Logic does) and classic v2 components (in process).
//
// Manual reference counting (built without ARC): nothing on the render path
// retains, releases or allocates. Calls marked "audio thread" are made from
// the render callback; every other call from the main thread, never at the
// same time as a render (src/audio/plugin.rs holds a lock around both).

#import <AVFoundation/AVFoundation.h>
#import <AppKit/AppKit.h>
#import <AudioToolbox/AudioToolbox.h>
#import <CoreAudioKit/CoreAudioKit.h>
#import <Foundation/Foundation.h>
#include <stdatomic.h>
#include <stdlib.h>
#include <string.h>

#define AB_MAX_CHANNELS 8

static char *ab_strdup_ns(NSString *s) {
    const char *utf8 = s ? s.UTF8String : "";
    if (!utf8) utf8 = "";
    size_t n = strlen(utf8);
    char *out = malloc(n + 1);
    memcpy(out, utf8, n + 1);
    return out;
}

void ab_free(void *p) { free(p); }

// ---------------------------------------------------------------------------
// Listing
// ---------------------------------------------------------------------------

static BOOL ab_hosted_type(OSType t) {
    return t == kAudioUnitType_Effect || t == kAudioUnitType_MusicDevice || t == kAudioUnitType_MusicEffect;
}

// Installed effects and instruments as a JSON array. Reads the system's
// registrations only: no plugin code runs.
char *ab_au_list(void) {
    @autoreleasepool {
        AudioComponentDescription any = {0};
        NSArray<AVAudioUnitComponent *> *all =
            [[AVAudioUnitComponentManager sharedAudioUnitComponentManager] componentsMatchingDescription:any];
        NSMutableArray *out = [NSMutableArray array];
        for (AVAudioUnitComponent *c in all) {
            AudioComponentDescription d = c.audioComponentDescription;
            if (!ab_hosted_type(d.componentType)) continue;
            AudioComponentDescription full = d;
            if (c.audioComponent) AudioComponentGetDescription(c.audioComponent, &full);
            BOOL v3 = (full.componentFlags & kAudioComponentFlag_IsV3AudioUnit) != 0;
            [out addObject:@{
                @"name" : c.name ?: @"",
                @"vendor" : c.manufacturerName ?: @"",
                @"type" : @(d.componentType),
                @"subtype" : @(d.componentSubType),
                @"manufacturer" : @(d.componentManufacturer),
                @"v3" : @(v3),
                @"midi" : @(c.hasMIDIInput),
                @"view" : @(c.hasCustomView),
            }];
        }
        NSData *json = [NSJSONSerialization dataWithJSONObject:out options:0 error:nil];
        char *text = malloc(json.length + 1);
        memcpy(text, json.bytes, json.length);
        text[json.length] = 0;
        return text;
    }
}

// ---------------------------------------------------------------------------
// The editor window
// ---------------------------------------------------------------------------

enum { AB_EDITOR_CLOSED = 0, AB_EDITOR_OPENING = 1, AB_EDITOR_OPEN = 2, AB_EDITOR_NONE = 3 };

// One editor request: outlives the unit if the view arrives after it closed.
@interface ABAUEditor : NSObject <NSWindowDelegate> {
  @public
    _Atomic int state;
    BOOL cancelled;
}
@property(nonatomic, retain) NSWindow *window;
/// For Apple's generic view, when the unit has none of its own.
@property(nonatomic, retain) AUAudioUnit *unit;
@end

@implementation ABAUEditor
- (void)windowWillClose:(NSNotification *)note {
    atomic_store(&state, AB_EDITOR_CLOSED);
    self.window.delegate = nil;
}
- (void)close {
    cancelled = YES;
    NSWindow *w = [self.window retain];
    self.window = nil;
    if (w) {
        w.delegate = nil;
        [w close];
        w.contentViewController = nil;
        [w release];
    }
    atomic_store(&state, AB_EDITOR_CLOSED);
}
- (void)dealloc {
    [_window release];
    [_unit release];
    [super dealloc];
}
@end

// ---------------------------------------------------------------------------
// A loaded unit
// ---------------------------------------------------------------------------

typedef struct ABTransport {
    double tempo;
    double beat;
    double beats_per_sample;
    double sample_pos;
    int playing;
    int changed;
} ABTransport;

typedef struct ABAudioUnit {
    AUAudioUnit *unit;
    AURenderBlock render;
    AURenderPullInputBlock pull;
    AUScheduleMIDIEventBlock midi;
    AUScheduleParameterBlock schedule_param;
    NSArray<AUParameter *> *params;
    /// Each parameter's address, for the audio thread.
    AUParameterAddress *addresses;
    ABAUEditor *editor;
    uint32_t in_channels, out_channels, max_frames;
    BOOL v3, has_view;
    // Render scratch: the input for the pull block, and the output.
    float *in_scratch[AB_MAX_CHANNELS];
    float *out_scratch[AB_MAX_CHANNELS];
    uint32_t pull_frames;
    AudioBufferList *out_list;
    ABTransport transport;
} ABAudioUnit;

static char *ab_error(NSString *what, NSError *e) {
    if (e) return ab_strdup_ns([NSString stringWithFormat:@"%@: %@ (%ld)", what, e.localizedDescription, (long)e.code]);
    return ab_strdup_ns(what);
}

// Sets bus `bus` to `channels` at `sr` (or, refused, its own channel count
// at `sr`). Returns the channels set, 0 on failure.
static uint32_t ab_set_bus(AUAudioUnitBus *bus, double sr, uint32_t channels) {
    NSError *e = nil;
    AVAudioFormat *f = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:sr channels:channels];
    BOOL ok = f && [bus setFormat:f error:&e];
    [f release];
    if (ok) return channels;
    uint32_t own = bus.format.channelCount;
    if (own == 0 || own > AB_MAX_CHANNELS) own = own == 0 ? 2 : AB_MAX_CHANNELS;
    f = [[AVAudioFormat alloc] initStandardFormatWithSampleRate:sr channels:own];
    ok = f && [bus setFormat:f error:&e];
    [f release];
    return ok ? own : 0;
}

void ab_au_close(ABAudioUnit *au);

// Loads and prepares a unit. v3 units are instantiated asynchronously by
// the system; this waits for them (up to 20 s). On failure returns NULL and
// sets `*error` (free with ab_free).
ABAudioUnit *ab_au_open(uint32_t type, uint32_t subtype, uint32_t manufacturer, double sample_rate, uint32_t max_frames,
                        char **error) {
    @autoreleasepool {
        *error = NULL;
        AudioComponentDescription d = {type, subtype, manufacturer, 0, 0};
        AudioComponent comp = AudioComponentFindNext(NULL, &d);
        if (!comp) {
            *error = ab_strdup_ns(@"not installed on this Mac");
            return NULL;
        }
        AudioComponentDescription full = d;
        AudioComponentGetDescription(comp, &full);
        BOOL v3 = (full.componentFlags & kAudioComponentFlag_IsV3AudioUnit) != 0;

        AUAudioUnit *unit = nil;
        NSError *err = nil;
        if (!v3) {
            // A v2 component: in process, at once.
            unit = [[AUAudioUnit alloc] initWithComponentDescription:d options:0 error:&err];
        } else {
            // An app extension: out of process (the system default on the
            // Mac), so a crash there can't take AudioBoard down. The answer
            // comes on a queue of the system's own.
            NSMutableArray *box = [[NSMutableArray alloc] init];
            dispatch_semaphore_t done = dispatch_semaphore_create(0);
            [AUAudioUnit instantiateWithComponentDescription:d
                                                     options:0
                                           completionHandler:^(AUAudioUnit *u, NSError *e) {
                                               @synchronized(box) {
                                                   [box addObject:u ?: (id)[NSNull null]];
                                                   [box addObject:e ?: (id)[NSNull null]];
                                               }
                                               dispatch_semaphore_signal(done);
                                           }];
            long waited = dispatch_semaphore_wait(done, dispatch_time(DISPATCH_TIME_NOW, 20 * NSEC_PER_SEC));
            dispatch_release(done);
            if (waited != 0) {
                [box release];
                *error = ab_strdup_ns(@"the Audio Unit did not start in time");
                return NULL;
            }
            @synchronized(box) {
                id u = box[0], e = box[1];
                if (u != [NSNull null]) unit = [u retain];
                if (e != [NSNull null]) err = [[e retain] autorelease];
            }
            [box release];
        }
        if (!unit) {
            *error = ab_error(@"could not load the Audio Unit", err);
            return NULL;
        }

        ABAudioUnit *au = calloc(1, sizeof(ABAudioUnit));
        au->unit = unit;
        au->v3 = v3;
        au->transport.tempo = 120.0;
        unit.maximumFramesToRender = max_frames;

        // Stereo where it can; the main buses only.
        if (unit.outputBusses.count > 0) {
            au->out_channels = ab_set_bus(unit.outputBusses[0], sample_rate, 2);
            unit.outputBusses[0].enabled = YES;
            for (NSUInteger b = 1; b < unit.outputBusses.count; b++) unit.outputBusses[b].enabled = NO;
        }
        if (au->out_channels == 0) {
            *error = ab_strdup_ns(@"no audio output this host can use");
            ab_au_close(au);
            return NULL;
        }
        if (unit.inputBusses.count > 0 && type != kAudioUnitType_MusicDevice) {
            au->in_channels = ab_set_bus(unit.inputBusses[0], sample_rate, 2);
            // Hosts enable the input buses they use (they start off).
            unit.inputBusses[0].enabled = au->in_channels > 0;
            for (NSUInteger b = 1; b < unit.inputBusses.count; b++) unit.inputBusses[b].enabled = NO;
        }

        // The host's clock, read during render (same thread as the render
        // call that set it).
        ABTransport *t = &au->transport;
        unit.musicalContextBlock = ^BOOL(double *tempo, double *num, NSInteger *den, double *beat, NSInteger *to_next,
                                         double *downbeat) {
            if (tempo) *tempo = t->tempo;
            if (num) *num = 4.0;
            if (den) *den = 4;
            if (beat) *beat = t->beat;
            if (to_next) {
                double left = ceil(t->beat) - t->beat;
                *to_next = t->beats_per_sample > 0.0 ? (NSInteger)(left / t->beats_per_sample) : 0;
            }
            if (downbeat) *downbeat = floor(t->beat / 4.0) * 4.0;
            return YES;
        };
        unit.transportStateBlock = ^BOOL(AUHostTransportStateFlags *flags, double *pos, double *cycle_start, double *cycle_end) {
            if (flags) *flags = (t->changed ? AUHostTransportStateChanged : 0) | (t->playing ? AUHostTransportStateMoving : 0);
            if (pos) *pos = t->sample_pos;
            if (cycle_start) *cycle_start = 0.0;
            if (cycle_end) *cycle_end = 0.0;
            return YES;
        };

        if (![unit allocateRenderResourcesAndReturnError:&err]) {
            *error = ab_error(@"could not prepare the Audio Unit", err);
            ab_au_close(au);
            return NULL;
        }
        au->max_frames = (uint32_t)unit.maximumFramesToRender;
        if (au->max_frames == 0) au->max_frames = 512;
        for (uint32_t c = 0; c < AB_MAX_CHANNELS; c++) {
            au->in_scratch[c] = calloc(au->max_frames, sizeof(float));
            au->out_scratch[c] = calloc(au->max_frames, sizeof(float));
        }
        au->out_list = calloc(1, sizeof(AudioBufferList) + AB_MAX_CHANNELS * sizeof(AudioBuffer));
        au->render = Block_copy(unit.renderBlock);
        au->midi = unit.scheduleMIDIEventBlock ? Block_copy(unit.scheduleMIDIEventBlock) : NULL;
        au->schedule_param = unit.scheduleParameterBlock ? Block_copy(unit.scheduleParameterBlock) : NULL;
        ABAudioUnit *self_ = au;
        au->pull = Block_copy(^AUAudioUnitStatus(AudioUnitRenderActionFlags *flags, const AudioTimeStamp *ts,
                                                 AUAudioFrameCount frames, NSInteger bus, AudioBufferList *data) {
            uint32_t n = frames < self_->max_frames ? frames : self_->max_frames;
            for (UInt32 b = 0; b < data->mNumberBuffers; b++) {
                uint32_t c = b < AB_MAX_CHANNELS ? b : AB_MAX_CHANNELS - 1;
                if (data->mBuffers[b].mData == NULL) {
                    data->mBuffers[b].mData = self_->in_scratch[c];
                } else {
                    memcpy(data->mBuffers[b].mData, self_->in_scratch[c], n * sizeof(float));
                }
                data->mBuffers[b].mDataByteSize = n * sizeof(float);
            }
            return noErr;
        });

        NSArray<AUParameter *> *params = unit.parameterTree.allParameters ?: @[];
        NSMutableArray *writable = [NSMutableArray array];
        for (AUParameter *p in params) {
            if (p.flags & kAudioUnitParameterFlag_IsWritable) [writable addObject:p];
        }
        au->params = [writable copy];
        au->addresses = calloc(writable.count + 1, sizeof(AUParameterAddress));
        for (NSUInteger i = 0; i < writable.count; i++) au->addresses[i] = ((AUParameter *)writable[i]).address;
        au->has_view = YES;
        return au;
    }
}

void ab_au_close(ABAudioUnit *au) {
    if (!au) return;
    @autoreleasepool {
        if (au->editor) {
            [au->editor close];
            [au->editor release];
        }
        if (au->unit) {
            if (au->render) [au->unit deallocateRenderResources];
            au->unit.musicalContextBlock = nil;
            au->unit.transportStateBlock = nil;
            [au->unit release];
        }
        if (au->render) Block_release(au->render);
        if (au->pull) Block_release(au->pull);
        if (au->midi) Block_release(au->midi);
        if (au->schedule_param) Block_release(au->schedule_param);
        [au->params release];
        free(au->addresses);
        for (uint32_t c = 0; c < AB_MAX_CHANNELS; c++) {
            free(au->in_scratch[c]);
            free(au->out_scratch[c]);
        }
        free(au->out_list);
        free(au);
    }
}

// What a loaded unit is: name, maker, channels, MIDI, latency.
void ab_au_info(ABAudioUnit *au, uint32_t *in_channels, uint32_t *out_channels, int *midi, int *v3, char **name,
                char **vendor) {
    @autoreleasepool {
        *in_channels = au->in_channels;
        *out_channels = au->out_channels;
        *midi = au->midi != NULL;
        *v3 = au->v3;
        *name = ab_strdup_ns(au->unit.audioUnitName ?: au->unit.componentName);
        *vendor = ab_strdup_ns(au->unit.manufacturerName);
    }
}

// ---------------------------------------------------------------------------
// Audio thread
// ---------------------------------------------------------------------------

// The host clock for the next render.
void ab_au_set_transport(ABAudioUnit *au, double tempo, double beat, double beats_per_sample, double sample_pos,
                         int playing, int changed) {
    ABTransport *t = &au->transport;
    t->tempo = tempo;
    t->beat = beat;
    t->beats_per_sample = beats_per_sample;
    t->sample_pos = sample_pos;
    t->playing = playing;
    t->changed = changed;
}

// A MIDI message `offset` frames into the next render.
void ab_au_midi(ABAudioUnit *au, int64_t offset, const uint8_t *bytes, uint32_t len) {
    if (au->midi) au->midi(AUEventSampleTimeImmediate + offset, 0, len, bytes);
}

// A parameter (by index) for the next render, in its own units.
void ab_au_schedule_param(ABAudioUnit *au, uint32_t index, uint32_t count, float value) {
    if (index >= count) return;
    if (au->schedule_param) {
        au->schedule_param(AUEventSampleTimeImmediate, 0, au->addresses[index], value);
    } else {
        au->params[index].value = value;
    }
}

// Renders `frames` (in pieces of the unit's largest render) from `inputs`
// (`in_count` channels, or none) into `outputs` (`out_count` channels).
// Returns 0, or the unit's error.
int ab_au_render(ABAudioUnit *au, uint32_t frames, const float *const *inputs, uint32_t in_count, float *const *outputs,
                 uint32_t out_count, int64_t sample_time) {
    uint32_t done = 0;
    while (done < frames) {
        uint32_t n = frames - done;
        if (n > au->max_frames) n = au->max_frames;
        for (uint32_t c = 0; c < au->in_channels && c < AB_MAX_CHANNELS; c++) {
            if (inputs && in_count > 0) {
                memcpy(au->in_scratch[c], inputs[c < in_count ? c : in_count - 1] + done, n * sizeof(float));
            } else {
                memset(au->in_scratch[c], 0, n * sizeof(float));
            }
        }
        AudioBufferList *abl = au->out_list;
        abl->mNumberBuffers = au->out_channels;
        for (uint32_t c = 0; c < au->out_channels; c++) {
            abl->mBuffers[c].mNumberChannels = 1;
            abl->mBuffers[c].mDataByteSize = n * sizeof(float);
            abl->mBuffers[c].mData = au->out_scratch[c];
        }
        AudioUnitRenderActionFlags flags = 0;
        AudioTimeStamp ts;
        memset(&ts, 0, sizeof(ts));
        ts.mSampleTime = (Float64)(sample_time + done);
        ts.mFlags = kAudioTimeStampSampleTimeValid;
        AUAudioUnitStatus status = au->render(&flags, &ts, n, 0, abl, au->pull);
        if (status != noErr) return (int)status;
        for (uint32_t c = 0; c < out_count; c++) {
            uint32_t src = c < au->out_channels ? c : au->out_channels - 1;
            const float *data = abl->mBuffers[src].mData;
            if (data) {
                memcpy(outputs[c] + done, data, n * sizeof(float));
            } else {
                memset(outputs[c] + done, 0, n * sizeof(float));
            }
        }
        au->transport.changed = 0;
        done += n;
    }
    return 0;
}

// ---------------------------------------------------------------------------
// Parameters (main thread)
// ---------------------------------------------------------------------------

uint32_t ab_au_param_count(ABAudioUnit *au) { return (uint32_t)au->params.count; }

// The parameters on their own, for setting them from the UI at any time
// (AUParameter is thread-safe, and the unit takes the value at once). Keep
// it no longer than the unit; release with ab_au_handle_release.
void *ab_au_param_handle(ABAudioUnit *au) { return [au->params retain]; }

void ab_au_handle_set(void *handle, uint32_t index, float value) {
    NSArray<AUParameter *> *params = handle;
    if (index >= params.count) return;
    @autoreleasepool {
        params[index].value = value;
    }
}

void ab_au_handle_release(void *handle) { [(NSArray *)handle release]; }

// Parameter `index`: its address, range, value now, flags and name (free
// with ab_free).
void ab_au_param_info(ABAudioUnit *au, uint32_t index, uint64_t *address, float *min, float *max, float *value,
                      uint32_t *flags, char **name) {
    @autoreleasepool {
        AUParameter *p = au->params[index];
        *address = p.address;
        *min = p.minValue;
        *max = p.maxValue;
        *value = p.value;
        *flags = p.flags;
        *name = ab_strdup_ns(p.displayName ?: p.identifier);
    }
}

float ab_au_get_param(ABAudioUnit *au, uint32_t index) {
    if (index >= au->params.count) return 0.0f;
    return au->params[index].value;
}

// The unit's own text for a value of parameter `index` (free with ab_free).
char *ab_au_format_param(ABAudioUnit *au, uint32_t index, float value) {
    @autoreleasepool {
        if (index >= au->params.count) return ab_strdup_ns(@"");
        AUParameter *p = au->params[index];
        NSString *text = nil;
        @try {
            text = [p stringFromValue:&value];
        } @catch (NSException *e) {
            text = nil;
        }
        NSString *unit = p.unitName;
        if (unit.length == 0) {
            switch (p.unit) {
                case kAudioUnitParameterUnit_Hertz: unit = @"Hz"; break;
                case kAudioUnitParameterUnit_Decibels: unit = @"dB"; break;
                case kAudioUnitParameterUnit_Percent:
                case kAudioUnitParameterUnit_EqualPowerCrossfade: unit = @"%"; break;
                case kAudioUnitParameterUnit_Seconds: unit = @"s"; break;
                case kAudioUnitParameterUnit_Milliseconds: unit = @"ms"; break;
                case kAudioUnitParameterUnit_Cents: unit = @"ct"; break;
                case kAudioUnitParameterUnit_RelativeSemiTones: unit = @"st"; break;
                case kAudioUnitParameterUnit_BPM: unit = @"BPM"; break;
                case kAudioUnitParameterUnit_Beats: unit = @"beats"; break;
                default: unit = @""; break;
            }
        }
        if (text.length == 0) text = [NSString stringWithFormat:@"%.3g", value];
        // A bare number gets its unit: "25 %", "1200 Hz".
        NSScanner *scan = [NSScanner scannerWithString:text];
        double number = 0;
        if (unit.length > 0 && [scan scanDouble:&number] && scan.isAtEnd) {
            text = [NSString stringWithFormat:@"%@ %@", text, unit];
        }
        return ab_strdup_ns(text);
    }
}

// ---------------------------------------------------------------------------
// State (main thread)
// ---------------------------------------------------------------------------

// The unit's complete state, as a binary property list (free with ab_free).
uint8_t *ab_au_save_state(ABAudioUnit *au, size_t *len) {
    @autoreleasepool {
        *len = 0;
        NSDictionary *state = au->unit.fullStateForDocument ?: au->unit.fullState;
        if (!state) return NULL;
        NSError *e = nil;
        NSData *data = [NSPropertyListSerialization dataWithPropertyList:state
                                                                  format:NSPropertyListBinaryFormat_v1_0
                                                                 options:0
                                                                   error:&e];
        if (!data) return NULL;
        uint8_t *out = malloc(data.length ?: 1);
        memcpy(out, data.bytes, data.length);
        *len = data.length;
        return out;
    }
}

// Restores a state from ab_au_save_state. Returns 0 on success.
int ab_au_load_state(ABAudioUnit *au, const uint8_t *bytes, size_t len) {
    @autoreleasepool {
        NSData *data = [NSData dataWithBytes:bytes length:len];
        NSError *e = nil;
        id plist = [NSPropertyListSerialization propertyListWithData:data options:NSPropertyListImmutable format:NULL error:&e];
        if (![plist isKindOfClass:[NSDictionary class]]) return -1;
        @try {
            au->unit.fullStateForDocument = plist;
        } @catch (NSException *ex) {
            return -2;
        }
        return 0;
    }
}

// ---------------------------------------------------------------------------
// Editor (main thread)
// ---------------------------------------------------------------------------

static void ab_show_editor(ABAUEditor *editor, NSViewController *vc, NSString *title) {
    if (editor->cancelled) return;
    NSViewController *generic = nil;
    if (!vc) {
        // No view of its own: Apple's generic one (sliders), where the
        // system has it.
        Class cls = NSClassFromString(@"AUGenericViewController");
        if (cls && editor.unit && [cls instancesRespondToSelector:@selector(setAuAudioUnit:)]) {
            generic = [[cls alloc] init];
            [generic performSelector:@selector(setAuAudioUnit:) withObject:editor.unit];
            vc = generic;
        }
    }
    editor.unit = nil;
    if (!vc) {
        atomic_store(&editor->state, AB_EDITOR_NONE);
        return;
    }
    NSView *view = vc.view;
    NSSize size = vc.preferredContentSize;
    if (size.width < 40 || size.height < 40) size = view.frame.size;
    if (size.width < 40 || size.height < 40) size = NSMakeSize(480, 320);
    NSWindow *w = [[NSWindow alloc] initWithContentRect:NSMakeRect(0, 0, size.width, size.height)
                                              styleMask:NSWindowStyleMaskTitled | NSWindowStyleMaskClosable |
                                                        NSWindowStyleMaskMiniaturizable
                                                backing:NSBackingStoreBuffered
                                                  defer:NO];
    w.releasedWhenClosed = NO;
    w.title = title;
    w.contentViewController = vc;
    [w setContentSize:size];
    // In front of the board while AudioBoard is active, hidden while
    // another app is, as plugin windows are in DAWs.
    w.level = NSFloatingWindowLevel;
    w.hidesOnDeactivate = YES;
    w.delegate = editor;
    editor.window = w;
    [w release];
    [w center];
    [w makeKeyAndOrderFront:nil];
    [generic release];
    atomic_store(&editor->state, AB_EDITOR_OPEN);
}

// Asks the unit for its view; the window opens when it comes (poll
// ab_au_editor_state).
void ab_au_open_editor(ABAudioUnit *au, const char *title) {
    @autoreleasepool {
        if (au->editor) {
            [au->editor close];
            [au->editor release];
        }
        ABAUEditor *editor = [[ABAUEditor alloc] init];
        editor.unit = au->unit;
        atomic_store(&editor->state, AB_EDITOR_OPENING);
        au->editor = editor;
        NSString *name = [NSString stringWithUTF8String:title ?: ""];
        [au->unit requestViewControllerWithCompletionHandler:^(AUViewControllerBase *vc) {
            [vc retain];
            dispatch_async(dispatch_get_main_queue(), ^{
                ab_show_editor(editor, vc, name);
                [vc release];
            });
        }];
    }
}

void ab_au_close_editor(ABAudioUnit *au) {
    if (!au->editor) return;
    @autoreleasepool {
        [au->editor close];
        [au->editor release];
        au->editor = nil;
    }
}

// 0 closed (or closed by the user), 1 opening, 2 open, 3 the unit has no view.
int ab_au_editor_state(ABAudioUnit *au) {
    if (!au->editor) return AB_EDITOR_CLOSED;
    return atomic_load(&au->editor->state);
}
