// Private pacing apparatus; no capture, input, activation, or permission APIs.
// Build: clang -fobjc-arc -fobjc-arc-exceptions -fblocks -mmacosx-version-min=26.5.2 fixture.m -framework AppKit -framework QuartzCore -o fixture
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#include <dispatch/dispatch.h>
#include <errno.h>
#include <fcntl.h>
#include <stdarg.h>
#include <stdbool.h>
#include <stdint.h>
#include <stdio.h>
#include <string.h>
#include <sys/mount.h>
#include <sys/resource.h>
#include <sys/stat.h>
#include <time.h>
#include <unistd.h>

#define SAMPLE_LIMIT 4096
#define JSON_LIMIT 131072
#define IMAGE_LIMIT (8 * 1024 * 1024)

typedef enum {
    ErrorNone, ErrorArguments, ErrorRoot, ErrorFile, ErrorCommand, ErrorSequence,
    ErrorForeground, ErrorImage, ErrorRender, ErrorGeometry, ErrorClock, ErrorOutput,
    ErrorClosed, ErrorNative, ErrorFuse, ErrorCleanup, ErrorException
} FixtureError;

static const char *reason(FixtureError error) {
    switch (error) {
    case ErrorNone: return "fixture-ok";
    case ErrorArguments: return "fixture-arguments-invalid";
    case ErrorRoot: return "fixture-root-invalid";
    case ErrorFile: return "fixture-file-invalid";
    case ErrorCommand: return "fixture-command-invalid";
    case ErrorSequence: return "fixture-sequence-invalid";
    case ErrorForeground: return "fixture-foreground-changed";
    case ErrorImage: return "fixture-image-invalid";
    case ErrorRender: return "fixture-render-failed";
    case ErrorGeometry: return "fixture-geometry-invalid";
    case ErrorClock: return "fixture-clock-failed";
    case ErrorOutput: return "fixture-output-failed";
    case ErrorClosed: return "fixture-window-closed";
    case ErrorNative: return "fixture-native-failed";
    case ErrorFuse: return "fixture-fuse-expired";
    case ErrorCleanup: return "fixture-cleanup-failed";
    case ErrorException: return "fixture-exception";
    }
    return "fixture-native-failed";
}

typedef enum { Command, Stop, Ack, AckTemp, Metrics, MetricsTemp, Failure, FailureTemp, FileCount } FileName;
static const char *const fileNames[FileCount] = {
    "command", "stop", "ack", "ack.tmp", "fixture-metrics.json",
    "fixture-metrics.json.tmp", "fixture-error", "fixture-error.tmp"
};

typedef struct { int root; bool ready; } Control;

static bool private_regular(const struct stat *attributes, uint64_t limit) {
    return S_ISREG(attributes->st_mode) && attributes->st_uid == geteuid() &&
        attributes->st_nlink == 1 && (attributes->st_mode & 07777) == 0600 &&
        attributes->st_size >= 0 && (uint64_t)attributes->st_size <= limit;
}

static bool open_control(Control *control, const char *path) {
    if (getuid() != geteuid() || strlen(path) > 2047) return false;
    control->root = open(path, O_RDONLY | O_DIRECTORY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
    struct stat attributes;
    struct statfs filesystem;
    if (control->root < 0 || fstat(control->root, &attributes) != 0 ||
        fstatfs(control->root, &filesystem) != 0 || !(filesystem.f_flags & MNT_LOCAL) ||
        !S_ISDIR(attributes.st_mode) || attributes.st_uid != geteuid() ||
        (attributes.st_mode & 07777) != 0700 || attributes.st_nlink == 0) return false;
    control->ready = true; // All subsequent paths are relative to this owned directory handle.
    return true;
}

static bool stop_requested(const Control *control, FixtureError *error) {
    struct stat attributes;
    if (fstatat(control->root, fileNames[Stop], &attributes, AT_SYMLINK_NOFOLLOW) == 0)
        return true; // Existence alone authorizes owned cleanup, regardless of file contents.
    if (errno != ENOENT) *error = ErrorFile;
    return false;
}

static bool outputs_absent(const Control *control) {
    for (size_t index = Ack; index < FileCount; ++index) {
        struct stat attributes;
        if (fstatat(control->root, fileNames[index], &attributes, AT_SYMLINK_NOFOLLOW) == 0 || errno != ENOENT)
            return false;
    }
    return true;
}

static bool atomic_write(const Control *control, FileName destination, FileName temporary,
                         const char *bytes, size_t length) {
    if (!control->ready || length > JSON_LIMIT) return false;
    struct stat attributes;
    if (fstatat(control->root, fileNames[destination], &attributes, AT_SYMLINK_NOFOLLOW) == 0) {
        if (!private_regular(&attributes, JSON_LIMIT)) return false;
    } else if (errno != ENOENT) return false;
    int file = openat(control->root, fileNames[temporary], O_WRONLY | O_CREAT | O_EXCL | O_NOFOLLOW | O_CLOEXEC, 0600);
    if (file < 0) return false;
    bool ok = fstat(file, &attributes) == 0 && private_regular(&attributes, 0);
    size_t offset = 0;
    while (ok && offset < length) {
        ssize_t written = write(file, bytes + offset, length - offset);
        if (written <= 0) ok = false;
        else offset += (size_t)written;
    }
    if (ok && fsync(file) != 0) ok = false;
    if (close(file) != 0) ok = false;
    if (ok && renameat(control->root, fileNames[temporary], control->root, fileNames[destination]) != 0) ok = false;
    if (!ok) unlinkat(control->root, fileNames[temporary], 0);
    return ok;
}

static void report_error(const Control *control, FixtureError error) {
    char row[64];
    int length = snprintf(row, sizeof(row), "%s\n", reason(error));
    if (length > 0 && (size_t)length < sizeof(row)) {
        atomic_write(control, Failure, FailureTemp, row, (size_t)length);
        fputs(row, stderr);
    }
}

static bool monotonic_ns(uint64_t *value) {
    struct timespec time;
    if (clock_gettime(CLOCK_MONOTONIC, &time) != 0 || time.tv_sec < 0 || time.tv_nsec < 0 ||
        time.tv_nsec >= 1000000000 || (uint64_t)time.tv_sec > (UINT64_MAX - (uint64_t)time.tv_nsec) / 1000000000)
        return false;
    *value = (uint64_t)time.tv_sec * 1000000000 + (uint64_t)time.tv_nsec;
    return true;
}

static bool timeval_ns(struct timeval time, uint64_t *value) {
    if (time.tv_sec < 0 || time.tv_usec < 0 || time.tv_usec >= 1000000 ||
        (uint64_t)time.tv_sec > (UINT64_MAX - (uint64_t)time.tv_usec * 1000) / 1000000000) return false;
    *value = (uint64_t)time.tv_sec * 1000000000 + (uint64_t)time.tv_usec * 1000;
    return true;
}

static bool cpu_time(uint64_t *value) {
    struct rusage usage;
    uint64_t user = 0, system = 0;
    if (getrusage(RUSAGE_SELF, &usage) != 0 || !timeval_ns(usage.ru_utime, &user) ||
        !timeval_ns(usage.ru_stime, &system) || system > UINT64_MAX - user) return false;
    *value = user + system;
    return true;
}

typedef struct {
    uint64_t wallStart, cpuStart, lastStart, lost;
    bool cpuAvailable, hasLastStart;
    uint64_t intervals[SAMPLE_LIMIT], durations[SAMPLE_LIMIT];
    size_t intervalCount, durationCount;
} Statistics;

static bool reset_statistics(Statistics *statistics) {
    if (!monotonic_ns(&statistics->wallStart)) return false;
    statistics->cpuAvailable = cpu_time(&statistics->cpuStart);
    statistics->hasLastStart = false;
    statistics->intervalCount = statistics->durationCount = 0;
    statistics->lost = 0;
    return true;
}

static bool append_sample(Statistics *statistics, uint64_t *samples, size_t *count, uint64_t value) {
    if (*count < SAMPLE_LIMIT) samples[(*count)++] = value;
    else {
        if (statistics->lost == UINT64_MAX) return false;
        ++statistics->lost;
    }
    return true;
}

static bool record_render(Statistics *statistics, uint64_t start, uint64_t end) {
    if (end < start || (statistics->hasLastStart && start < statistics->lastStart)) return false;
    if (statistics->hasLastStart &&
        !append_sample(statistics, statistics->intervals, &statistics->intervalCount, start - statistics->lastStart))
        return false;
    statistics->lastStart = start;
    statistics->hasLastStart = true;
    return append_sample(statistics, statistics->durations, &statistics->durationCount, end - start);
}

typedef struct { char bytes[JSON_LIMIT]; size_t length; bool valid; } Json;

static void json_append(Json *json, const char *format, ...) {
    if (!json->valid) return;
    va_list arguments;
    va_start(arguments, format);
    int added = vsnprintf(json->bytes + json->length, sizeof(json->bytes) - json->length, format, arguments);
    va_end(arguments);
    if (added < 0 || (size_t)added >= sizeof(json->bytes) - json->length) json->valid = false;
    else json->length += (size_t)added;
}

static void json_number(Json *json, bool available, uint64_t value) {
    if (available) json_append(json, "%llu", (unsigned long long)value);
    else json_append(json, "null");
}

typedef enum { OpBlank, OpShow, OpPulse, OpAnimate, OpPause, OpBurst, OpResize, OpResetStats, OpStats, OpClose, OpQuit } Operation;
static const char *const operations[] = {
    "blank", "show", "pulse", "animate", "pause", "burst", "resize", "reset-stats", "stats", "close-window", "quit"
};

static NSImage *load_image(const char *path) {
    int file = open(path, O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
    if (file < 0) return nil;
    struct stat attributes;
    bool valid = fstat(file, &attributes) == 0 && S_ISREG(attributes.st_mode) && attributes.st_nlink == 1 &&
        attributes.st_size >= 33 && attributes.st_size <= IMAGE_LIMIT;
    NSMutableData *data = nil;
    @try {
        data = valid ? [NSMutableData dataWithLength:(NSUInteger)attributes.st_size] : nil;
        valid = valid && data != nil;
        size_t offset = 0;
        while (valid && offset < data.length) {
            ssize_t count = read(file, (unsigned char *)data.mutableBytes + offset, data.length - offset);
            if (count <= 0) valid = false;
            else offset += (size_t)count;
        }
    } @finally {
        if (close(file) != 0) valid = false;
    }
    if (!valid) return nil;
    // Bound PNG dimensions before asking AppKit to decode the repository asset.
    static const unsigned char signature[] = {137, 80, 78, 71, 13, 10, 26, 10, 0, 0, 0, 13, 'I', 'H', 'D', 'R'};
    const unsigned char *header = data.bytes;
    if (memcmp(header, signature, sizeof(signature)) || header[16] != 0 || header[17] != 0 ||
        header[18] != 3 || header[19] != 192 || header[20] != 0 || header[21] != 0 ||
        header[22] != 2 || header[23] != 28) return nil;
    NSBitmapImageRep *representation = [NSBitmapImageRep imageRepWithData:data];
    if (!representation || representation.pixelsWide != 960 || representation.pixelsHigh != 540) return nil;
    representation.size = NSMakeSize(480, 270);
    NSImage *image = [[NSImage alloc] initWithSize:NSMakeSize(480, 270)];
    [image addRepresentation:representation];
    return image;
}

@interface OwnedWindow : NSPanel
@end
@implementation OwnedWindow
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
@end

@interface FixtureView : NSView {
    NSColor *_oneColor, *_zeroColor;
}
@property(nonatomic, strong) NSImage *image;
@property(nonatomic) uint64_t token;
@property(nonatomic) uint32_t counter;
@property(nonatomic) uint32_t state;
@property(nonatomic) BOOL didDraw;
@property(nonatomic) BOOL drawFailed;
@end

@implementation FixtureView
- (instancetype)initWithFrame:(NSRect)frame {
    self = [super initWithFrame:frame];
    if (self) {
        _oneColor = [NSColor colorWithSRGBRed:1 green:1 blue:0 alpha:1];
        _zeroColor = [NSColor colorWithSRGBRed:0 green:0 blue:1 alpha:1];
    }
    return self;
}
- (BOOL)isFlipped { return YES; }
- (BOOL)isOpaque { return YES; }
- (void)drawRect:(NSRect)dirty {
    (void)dirty;
    if (!self.counter) return;
    @try {
        NSGraphicsContext *context = NSGraphicsContext.currentContext;
        if (!context || self.window.backingScaleFactor != 2.0 || self.state > 1) {
            self.drawFailed = YES;
            return;
        }
        context.shouldAntialias = NO;
        context.imageInterpolation = NSImageInterpolationNone;
        [[NSColor whiteColor] setFill];
        NSRectFill(self.bounds);
        if (self.state == 1) {
            [self.image drawInRect:NSMakeRect(0, 0, 480, 270)
                         fromRect:NSZeroRect operation:NSCompositingOperationCopy
                         fraction:1.0 respectFlipped:YES hints:nil];
        }
        for (unsigned bit = 0; bit < 128; ++bit) {
            BOOL one = bit < 64 ? ((self.token >> (63 - bit)) & 1) != 0 :
                bit < 96 ? ((self.counter >> (95 - bit)) & 1) != 0 : ((self.state >> (127 - bit)) & 1) != 0;
            [(one ? _oneColor : _zeroColor) setFill];
            NSRectFill(NSMakeRect(bit * 2, 272, 2, 8));
        }
        self.didDraw = YES;
    } @catch (__unused NSException *exception) {
        self.drawFailed = YES;
    }
}
@end

@interface Fixture : NSObject <NSApplicationDelegate, NSWindowDelegate> {
    Control _control;
    Statistics _statistics;
    char _nonce[17];
    uint64_t _nonceBits, _windowId;
    uint32_t _sequence, _width, _height;
    Operation _lastOperation;
    FixtureError _error;
    pid_t _foreground;
    unsigned _burstRemaining;
    BOOL _done, _ownedClose;
    OwnedWindow *_window;
    FixtureView *_view;
    NSTimer *_animationTimer;
    id _foregroundObserver;
    dispatch_source_t _fuse;
    dispatch_queue_t _fuseQueue;
    dispatch_semaphore_t _fuseFinished;
}
- (void)fail:(FixtureError)error;
- (void)prepare:(int)argc arguments:(const char **)argv;
- (void)run;
- (int)finish;
- (void)emergencyExit;
@end

@implementation Fixture
- (instancetype)init {
    self = [super init];
    if (self) {
        _control.root = -1;
        _width = 960;
        _height = 576;
    }
    return self;
}

- (void)fail:(FixtureError)error {
    if (_error == ErrorNone) _error = error;
    _done = YES;
}

- (BOOL)foregroundUnchanged {
    if (_foreground > 0 && !NSApp.active &&
        NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier == _foreground) return YES;
    [self fail:ErrorForeground];
    return NO;
}

- (BOOL)geometryValid {
    NSRect backing = [_view convertRectToBacking:_view.bounds];
    // Flipped views can have a negative backing origin; frame size uses only its extent.
    if (_window && _window.backingScaleFactor == 2.0 &&
        backing.size.width == _width && backing.size.height == _height &&
        (!_windowId || _window.windowNumber == (NSInteger)_windowId)) return YES;
    [self fail:ErrorGeometry];
    return NO;
}

- (BOOL)startFuse {
    int root = fcntl(_control.root, F_DUPFD_CLOEXEC, 0);
    if (root < 0) return NO;
    _fuseQueue = dispatch_queue_create("MadoPilot.Private.CapturePacing.Fuse", DISPATCH_QUEUE_SERIAL);
    _fuseFinished = dispatch_semaphore_create(0);
    if (!_fuseQueue || !_fuseFinished) { close(root); return NO; }
    _fuse = dispatch_source_create(DISPATCH_SOURCE_TYPE_TIMER, 0, 0, _fuseQueue);
    if (!_fuse) { close(root); return NO; }
    dispatch_semaphore_t finished = _fuseFinished;
    dispatch_source_set_event_handler(_fuse, ^{
        Control control = {.root = root, .ready = true};
        report_error(&control, ErrorFuse);
        _exit(124);
    });
    dispatch_source_set_cancel_handler(_fuse, ^{
        close(root);
        dispatch_semaphore_signal(finished);
    });
    dispatch_source_set_timer(_fuse, dispatch_time(DISPATCH_TIME_NOW, 180 * NSEC_PER_SEC), DISPATCH_TIME_FOREVER, 0);
    dispatch_resume(_fuse);
    return YES;
}

- (void)stopAnimation {
    [_animationTimer invalidate];
    _animationTimer = nil;
    _burstRemaining = 0;
}

- (BOOL)render:(BOOL)first {
    if (!_window) { [self fail:ErrorClosed]; return NO; }
    if (![self foregroundUnchanged] || ![self geometryValid]) return NO;
    if (_view.counter == UINT32_MAX) { [self fail:ErrorRender]; return NO; }
    uint64_t start = 0, end = 0;
    if (!monotonic_ns(&start)) { [self fail:ErrorClock]; return NO; }
    _view.counter += 1;
    _view.didDraw = NO;
    if (first) [_window orderFrontRegardless]; // Nonactivating panel; never key/main.
    _view.needsDisplay = YES;
    [_view displayIfNeeded];
    [CATransaction flush];
    if (!_view.didDraw || _view.drawFailed) [self fail:ErrorRender];
    // Requested application renders only; exposure restores the unchanged token.
    if (!monotonic_ns(&end) || !record_render(&_statistics, start, end)) [self fail:ErrorClock];
    return _error == ErrorNone && [self foregroundUnchanged] && [self geometryValid];
}

- (BOOL)acknowledge {
    if (_error != ErrorNone || ![self foregroundUnchanged] || (_window && ![self geometryValid])) return NO;
    char row[512];
    int length = snprintf(row, sizeof(row), "ACK %s %u %u %llu %u %u %u %u\n",
        _nonce, _sequence, (unsigned)getpid(), (unsigned long long)_windowId,
        _width, _height, _view.counter, _view.state);
    if (length <= 0 || (size_t)length >= sizeof(row) || !atomic_write(&_control, Ack, AckTemp, row, (size_t)length))
        [self fail:ErrorOutput];
    return _error == ErrorNone;
}

- (BOOL)writeStatistics {
    uint64_t now = 0, cpu = 0;
    if (!monotonic_ns(&now) || now < _statistics.wallStart) { [self fail:ErrorClock]; return NO; }
    bool cpuAvailable = _statistics.cpuAvailable && cpu_time(&cpu) && cpu >= _statistics.cpuStart;
    struct rusage usage = {0};
    bool rssAvailable = getrusage(RUSAGE_SELF, &usage) == 0 && usage.ru_maxrss >= 0;
    // Darwin reports the process lifetime RSS high-water in bytes, not window memory.
    Json json;
    json.length = 0;
    json.valid = true;
    json_append(&json, "{\"schema\":1,\"nonce\":\"%s\",\"pid\":%u,\"wall_ns\":%llu,\"cpu_ns\":",
        _nonce, (unsigned)getpid(), (unsigned long long)(now - _statistics.wallStart));
    json_number(&json, cpuAvailable, cpuAvailable ? cpu - _statistics.cpuStart : 0);
    json_append(&json, ",\"peak_rss_bytes\":");
    json_number(&json, rssAvailable, rssAvailable ? (uint64_t)usage.ru_maxrss : 0);
    json_append(&json, ",\"paint_counter\":%u,\"render_intervals_ns\":[", _view.counter);
    for (size_t index = 0; index < _statistics.intervalCount; ++index)
        json_append(&json, "%s%llu", index ? "," : "", (unsigned long long)_statistics.intervals[index]);
    json_append(&json, "],\"draw_durations_ns\":[");
    for (size_t index = 0; index < _statistics.durationCount; ++index)
        json_append(&json, "%s%llu", index ? "," : "", (unsigned long long)_statistics.durations[index]);
    json_append(&json, "],\"lost_samples\":%llu}\n", (unsigned long long)_statistics.lost);
    if (!json.valid || !atomic_write(&_control, Metrics, MetricsTemp, json.bytes, json.length)) [self fail:ErrorOutput];
    return _error == ErrorNone;
}

- (BOOL)readCommand:(Operation *)operation sequence:(uint32_t *)next present:(BOOL *)present {
    *present = NO;
    int file = openat(_control.root, fileNames[Command], O_RDONLY | O_NOFOLLOW | O_CLOEXEC | O_NONBLOCK);
    if (file < 0) {
        if (errno != ENOENT) [self fail:ErrorFile];
        return _error == ErrorNone;
    }
    struct stat attributes;
    bool inspected = fstat(file, &attributes) == 0;
    // An atomic replacement can unlink the snapshot just opened. Read the new path next poll.
    if (inspected && attributes.st_nlink == 0) {
        if (close(file) != 0) [self fail:ErrorFile];
        return _error == ErrorNone;
    }
    bool valid = inspected && private_regular(&attributes, 128) && attributes.st_size > 0;
    char row[129];
    ssize_t count = valid ? pread(file, row, sizeof(row), 0) : -1;
    if (close(file) != 0) valid = false;
    if (!valid || count != attributes.st_size) { [self fail:ErrorFile]; return NO; }
    if (count < 21 || row[count - 1] != '\n' || memcmp(row, _nonce, 16) || row[16] != ' ') {
        [self fail:ErrorCommand]; return NO;
    }
    for (ssize_t index = 0; index + 1 < count; ++index) {
        if (row[index] < ' ' || row[index] > '~') { [self fail:ErrorCommand]; return NO; }
    }
    size_t offset = 17;
    *next = 0;
    if (row[offset] < '1' || row[offset] > '9') { [self fail:ErrorSequence]; return NO; }
    while (offset < (size_t)count && row[offset] >= '0' && row[offset] <= '9') {
        *next = *next * 10 + (unsigned)(row[offset++] - '0');
        if (*next > 128) { [self fail:ErrorSequence]; return NO; }
    }
    if (offset >= (size_t)count || row[offset++] != ' ') { [self fail:ErrorCommand]; return NO; }
    row[count - 1] = '\0';
    BOOL known = NO;
    for (size_t index = 0; index < sizeof(operations) / sizeof(operations[0]); ++index) {
        if (!strcmp(row + offset, operations[index])) {
            *operation = (Operation)index;
            known = YES;
            break;
        }
    }
    if (!known) { [self fail:ErrorCommand]; return NO; }
    if (*next == _sequence) {
        if (*operation != _lastOperation) [self fail:ErrorSequence];
        return _error == ErrorNone;
    }
    if (*next != _sequence + 1) { [self fail:ErrorSequence]; return NO; }
    *present = YES;
    return YES;
}

- (void)closeWindow {
    [self stopAnimation];
    if (!_window) return;
    _ownedClose = YES;
    [_window orderOut:nil];
    [_window close];
    if (_window.visible) [self fail:ErrorCleanup];
    _window.delegate = nil;
    _window = nil;
    _ownedClose = NO;
}

- (BOOL)execute:(Operation)operation sequence:(uint32_t)next {
    _sequence = next;
    _lastOperation = operation;
    BOOL repaint = NO;
    switch (operation) {
    case OpBlank: _view.state = 0; repaint = YES; break;
    case OpShow: _view.state = 1; repaint = YES; break;
    case OpPulse: repaint = YES; break;
    case OpAnimate:
    case OpBurst:
        [self stopAnimation];
        _view.state = 1;
        repaint = YES;
        break;
    case OpPause: [self stopAnimation]; break;
    case OpResize:
        if (!_window) { [self fail:ErrorClosed]; return NO; }
        [self stopAnimation];
        _width = 1040;
        _height = 640;
        [_window setContentSize:NSMakeSize(520, 320)];
        [_view setFrameSize:NSMakeSize(520, 320)];
        _view.state = 1;
        repaint = YES;
        break;
    case OpResetStats:
        if (!reset_statistics(&_statistics)) { [self fail:ErrorClock]; return NO; }
        break;
    case OpStats: if (![self writeStatistics]) return NO; break;
    case OpClose: [self closeWindow]; break;
    case OpQuit: [self stopAnimation]; _done = YES; break;
    }
    if ((repaint && ![self render:NO]) || ![self acknowledge]) return NO;
    if (operation == OpAnimate || operation == OpBurst) {
        _burstRemaining = operation == OpBurst ? 8 : 0;
        __weak Fixture *weakSelf = self;
        // The timer is created only after ACK; all eight burst events are post-ACK.
        _animationTimer = [NSTimer timerWithTimeInterval:0.016 repeats:YES block:^(NSTimer *timer) {
            (void)timer;
            Fixture *owner = weakSelf;
            if (!owner || owner->_done) return;
            @try {
                if (![owner render:NO]) return;
                if (owner->_burstRemaining && --owner->_burstRemaining == 0) [owner stopAnimation];
            } @catch (__unused NSException *exception) {
                [owner fail:ErrorException];
            }
        }];
        if (!_animationTimer) { [self fail:ErrorNative]; return NO; }
        [NSRunLoop.mainRunLoop addTimer:_animationTimer forMode:NSDefaultRunLoopMode];
    }
    return YES;
}

- (void)poll {
    if (_done) return;
    FixtureError fileError = ErrorNone;
    if (stop_requested(&_control, &fileError)) { _done = YES; return; }
    if (fileError != ErrorNone) { [self fail:fileError]; return; }
    if (![self foregroundUnchanged] || (_window && ![self geometryValid])) return;
    if (_view.drawFailed) { [self fail:ErrorRender]; return; }
    Operation operation = OpBlank;
    uint32_t next = 0;
    BOOL present = NO;
    if (![self readCommand:&operation sequence:&next present:&present]) return;
    if (present) [self execute:operation sequence:next];
}

- (void)prepare:(int)argc arguments:(const char **)argv {
    if (argc != 4) { [self fail:ErrorArguments]; return; }
    if (!open_control(&_control, argv[1])) { [self fail:ErrorRoot]; return; }
    if (![self startFuse]) { [self fail:ErrorNative]; return; }
    FixtureError fileError = ErrorNone;
    if (stop_requested(&_control, &fileError)) { _done = YES; return; }
    if (fileError != ErrorNone) { [self fail:fileError]; return; }
    if (!outputs_absent(&_control)) { [self fail:ErrorFile]; return; }
    if (strlen(argv[2]) != 16) { [self fail:ErrorArguments]; return; }
    for (size_t index = 0; index < 16; ++index) {
        char character = argv[2][index];
        int digit = character >= '0' && character <= '9' ? character - '0' :
            character >= 'a' && character <= 'f' ? character - 'a' + 10 : -1;
        if (digit < 0) { [self fail:ErrorArguments]; return; }
        _nonce[index] = character;
        _nonceBits = (_nonceBits << 4) | (unsigned)digit;
    }
#if !defined(__arm64__)
    [self fail:ErrorNative];
    return;
#endif
    _foreground = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
    if (_foreground <= 0 || _foreground == getpid()) { [self fail:ErrorForeground]; return; }
    if (!reset_statistics(&_statistics)) { [self fail:ErrorClock]; return; }
    [NSApplication sharedApplication];
    // Preserve the CLI's nonactivating policy; promoting to Accessory can take foreground.
    if (NSApp.activationPolicy != NSApplicationActivationPolicyProhibited) { [self fail:ErrorNative]; return; }
    NSApp.delegate = self;
    __weak Fixture *weakSelf = self;
    _foregroundObserver = [NSWorkspace.sharedWorkspace.notificationCenter
        addObserverForName:NSWorkspaceDidActivateApplicationNotification object:nil queue:NSOperationQueue.mainQueue
        usingBlock:^(NSNotification *notification) {
            Fixture *owner = weakSelf;
            if (owner && !owner->_done) {
                NSRunningApplication *activated = notification.userInfo[NSWorkspaceApplicationKey];
                if (!activated || activated.processIdentifier != owner->_foreground) [owner fail:ErrorForeground];
                else [owner foregroundUnchanged];
            }
        }];
    [NSApp finishLaunching];
    if (_done || ![self foregroundUnchanged]) return;
    NSImage *image = load_image(argv[3]);
    if (!image) { [self fail:ErrorImage]; return; }
    NSScreen *screen = NSScreen.mainScreen;
    if (!screen || screen.backingScaleFactor != 2.0 ||
        screen.visibleFrame.size.width < 600 || screen.visibleFrame.size.height < 400) {
        [self fail:ErrorGeometry]; return;
    }
    NSRect box = NSMakeRect(screen.visibleFrame.origin.x + 40, screen.visibleFrame.origin.y + 40, 480, 288);
    _window = [[OwnedWindow alloc] initWithContentRect:box
        styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
        backing:NSBackingStoreBuffered defer:NO screen:screen];
    if (!_window) { [self fail:ErrorNative]; return; }
    _window.releasedWhenClosed = NO;
    _window.hidesOnDeactivate = NO;
    _window.floatingPanel = NO;
    _window.level = NSNormalWindowLevel;
    _window.hasShadow = NO;
    _window.opaque = YES;
    _window.backgroundColor = NSColor.whiteColor;
    _window.colorSpace = NSColorSpace.sRGBColorSpace;
    _window.ignoresMouseEvents = YES;
    _window.movable = NO;
    _window.delegate = self;
    NSString *nonce = [NSString stringWithUTF8String:_nonce];
    _window.title = [@"MadoPilot Pacing " stringByAppendingString:nonce];
    _view = [[FixtureView alloc] initWithFrame:NSMakeRect(0, 0, 480, 288)];
    if (!_view) { [self fail:ErrorNative]; return; }
    _view.autoresizingMask = NSViewWidthSizable | NSViewHeightSizable;
    _view.image = image;
    _view.token = _nonceBits;
    _window.contentView = _view;
    if (![self render:YES]) return;
    NSInteger number = _window.windowNumber;
    if (number <= 0) { [self fail:ErrorNative]; return; }
    _windowId = (uint64_t)number;
    [self acknowledge];
}

- (void)run {
    // A bounded AppKit loop avoids NSApp termination/last-window semantics and stop-event wakeups.
    while (!_done) {
        @autoreleasepool {
            [self poll]; // Queued GUI events cannot starve command/stop processing.
            if (_done) break;
            NSEvent *event = [NSApp nextEventMatchingMask:NSEventMaskAny
                untilDate:[NSDate dateWithTimeIntervalSinceNow:0.008]
                inMode:NSDefaultRunLoopMode dequeue:YES];
            if (event) [NSApp sendEvent:event];
            [NSApp updateWindows];
        }
    }
}

- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)application {
    (void)application;
    return NO;
}

- (NSApplicationTerminateReply)applicationShouldTerminate:(NSApplication *)application {
    (void)application;
    [self fail:ErrorNative];
    return NSTerminateCancel;
}

- (BOOL)windowShouldClose:(NSWindow *)sender {
    (void)sender;
    if (!_ownedClose) [self fail:ErrorClosed];
    return _ownedClose;
}

- (void)windowWillClose:(NSNotification *)notification {
    (void)notification;
    if (!_ownedClose) [self fail:ErrorClosed];
}

- (int)finish {
    _done = YES;
    [self closeWindow];
    if (_foregroundObserver) {
        [NSWorkspace.sharedWorkspace.notificationCenter removeObserver:_foregroundObserver];
        _foregroundObserver = nil;
    }
    if (_foreground > 0) [self foregroundUnchanged];
    NSApp.delegate = nil;
    _view = nil;
    if (_fuse) {
        dispatch_source_cancel(_fuse);
        if (dispatch_semaphore_wait(_fuseFinished, dispatch_time(DISPATCH_TIME_NOW, 5 * NSEC_PER_SEC)) != 0) {
            report_error(&_control, ErrorCleanup);
            _exit(1);
        }
        _fuse = nil;
        _fuseQueue = nil;
        _fuseFinished = nil;
    }
    if (_error != ErrorNone) report_error(&_control, _error);
    if (_control.root >= 0 && close(_control.root) != 0) {
        fputs("fixture-cleanup-failed\n", stderr);
        if (_error == ErrorNone) _error = ErrorCleanup;
    }
    _control.root = -1;
    _control.ready = false;
    return _error == ErrorNone ? 0 : (_error == ErrorArguments || _error == ErrorRoot ? 2 : 1);
}

- (void)emergencyExit {
    report_error(&_control, ErrorCleanup);
    _exit(1);
}
@end

int main(int argc, const char *argv[]) {
    @autoreleasepool {
        // NSApp's delegate is not an owner; keep the timer target alive through complete cleanup.
        __attribute__((objc_precise_lifetime)) Fixture *owner = [Fixture new];
        if (!owner) { fputs("fixture-native-failed\n", stderr); return 1; }
        @try {
            [owner prepare:argc arguments:argv];
            [owner run];
        } @catch (__unused NSException *exception) {
            [owner fail:ErrorException];
        }
        @try { return [owner finish]; }
        @catch (__unused NSException *exception) {
            [owner emergencyExit];
        }
    }
}
