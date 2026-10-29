// Private task 7.3 apparatus. No capture, permission, input, or activation APIs.
#import <AppKit/AppKit.h>
#import <QuartzCore/QuartzCore.h>
#include <stdint.h>
#include <string.h>
#include <unistd.h>

@interface OwnedWindow : NSPanel
@end
@implementation OwnedWindow
- (BOOL)canBecomeKeyWindow { return NO; }
- (BOOL)canBecomeMainWindow { return NO; }
@end

@interface FixtureView : NSView
@property(nonatomic, strong) NSImage *image;
@property(nonatomic) uint64_t token;
@property(nonatomic) uint32_t state;
@property(nonatomic) BOOL showImage;
@end
@implementation FixtureView
- (BOOL)isFlipped { return YES; }
- (BOOL)isOpaque { return YES; }
- (void)drawRect:(NSRect)dirty {
    (void)dirty;
    [[NSColor whiteColor] setFill];
    NSRectFill(self.bounds);
    if (self.showImage) {
        [NSGraphicsContext currentContext].imageInterpolation = NSImageInterpolationNone;
        [self.image drawInRect:NSMakeRect(0, 0, 480, 270)
                     fromRect:NSZeroRect operation:NSCompositingOperationCopy
                     fraction:1.0 respectFlipped:YES hints:nil];
    }
    // Outside the fixed OCR ROI: 64 ownership bits, then 32 transition bits.
    // Each cell is 2x8 logical points (4x16 backing pixels at the required 2x).
    for (unsigned bit = 0; bit < 96; ++bit) {
        BOOL one = bit < 64 ? ((self.token >> (63 - bit)) & 1)
                            : ((self.state >> (95 - bit)) & 1);
        // Neutral endpoints remain neutral in ScreenCaptureKit's display color space.
        [(one ? NSColor.whiteColor : NSColor.blackColor) setFill];
        NSRectFill(NSMakeRect(bit * 2, self.bounds.size.height - 8, 2, 8));
    }
}
@end

@interface Fixture : NSObject <NSApplicationDelegate>
@property(nonatomic, strong) OwnedWindow *window;
@property(nonatomic, strong) FixtureView *view;
@property(nonatomic, strong) NSImage *image;
@property(nonatomic, strong) NSScreen *screen;
@property(nonatomic, strong) NSArray<NSString *> *steps;
@property(nonatomic, strong) NSURL *root;
@property(nonatomic, copy) NSString *nonce;
@property(nonatomic) uint64_t token;
@property(nonatomic) uint32_t sequence;
@property(nonatomic) pid_t originalFrontmost;
@property(nonatomic) NSInteger ownedWindowNumber;
@property(nonatomic) CGFloat renderedScale;
@property(nonatomic) NSSize renderedSize;
@property(nonatomic, strong) NSTimer *timer;
- (BOOL)acknowledge:(NSString *)kind;
@end

@implementation Fixture
- (BOOL)acknowledge:(NSString *)kind {
    pid_t frontmost = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
    if (NSApp.active || frontmost != self.originalFrontmost) return NO;
    NSString *row = [NSString stringWithFormat:@"%@ %u %@ %d %ld %.0f %.0f %.1f\n",
        self.nonce, self.sequence, kind, getpid(), (long)self.ownedWindowNumber,
        self.renderedSize.width, self.renderedSize.height,
        self.renderedScale];
    return [row writeToURL:[self.root URLByAppendingPathComponent:@"ack"]
               atomically:YES encoding:NSUTF8StringEncoding error:NULL];
}
- (BOOL)render:(NSString *)kind {
    if (self.window.backingScaleFactor != 2.0) return NO;
    self.ownedWindowNumber = self.window.windowNumber;
    self.renderedScale = self.window.backingScaleFactor;
    self.renderedSize = self.view.bounds.size;
    self.view.state = self.sequence;
    self.view.needsDisplay = YES;
    [self.view displayIfNeeded];
    [CATransaction flush];
    // AppKit display processing returned and pending transactions were submitted.
    // This is not a GPU/WindowServer completion fence or a ScreenCaptureKit frame;
    // the consumer independently requires this state's pixels on its source.
    return [self acknowledge:kind];
}
- (void)fail {
    [@"fixture-failed\n" writeToURL:[self.root URLByAppendingPathComponent:@"fixture-failure"]
        atomically:YES encoding:NSUTF8StringEncoding error:NULL];
    [NSApp terminate:nil];
}
// One owned 480x278 window at the fixed initial position. Startup and the
// `open-window` step both use it, so a replacement window is a fresh AppKit
// window and WindowServer identity, never a reordered or unhidden old one.
- (void)createWindow {
    NSRect box = NSMakeRect(self.screen.visibleFrame.origin.x + 40, self.screen.visibleFrame.origin.y + 40, 480, 278);
    self.window = [[OwnedWindow alloc] initWithContentRect:box
        styleMask:(NSWindowStyleMaskBorderless | NSWindowStyleMaskNonactivatingPanel)
        backing:NSBackingStoreBuffered defer:NO screen:self.screen];
    self.window.releasedWhenClosed = NO;
    self.window.hidesOnDeactivate = NO;
    self.window.floatingPanel = NO;
    self.window.level = NSNormalWindowLevel;
    self.window.hasShadow = NO;
    self.window.opaque = YES;
    self.window.backgroundColor = NSColor.whiteColor;
    self.window.title = [@"MadoPilot OCR private " stringByAppendingString:self.nonce];
    self.window.ignoresMouseEvents = YES;
    self.view = [[FixtureView alloc] initWithFrame:NSMakeRect(0, 0, 480, 278)];
    self.view.image = self.image;
    self.view.token = self.token;
    self.window.contentView = self.view;
    [self.window orderFrontRegardless]; // Never make key/main or activate.
}
- (void)applicationDidFinishLaunching:(NSNotification *)notification {
    (void)notification;
    [self createWindow];
    if (![self render:@"blank"]) { [self fail]; return; }
    __weak Fixture *weakSelf = self;
    self.timer = [NSTimer scheduledTimerWithTimeInterval:0.02 repeats:YES block:^(NSTimer *timer) {
        (void)timer;
        Fixture *owner = weakSelf;
        if (!owner) return;
        if ([NSFileManager.defaultManager fileExistsAtPath:[owner.root URLByAppendingPathComponent:@"stop"].path]) {
            [owner.window orderOut:nil];
            [owner.window close];
            [NSApp terminate:nil];
            return;
        }
        NSURL *command = [owner.root URLByAppendingPathComponent:@"command"];
        NSDictionary *attributes = [NSFileManager.defaultManager attributesOfItemAtPath:command.path error:NULL];
        if (!attributes) return;
        if ([attributes fileSize] > 256) { [owner fail]; return; }
        NSString *line = [NSString stringWithContentsOfURL:command encoding:NSUTF8StringEncoding error:NULL];
        if (!line || [line lengthOfBytesUsingEncoding:NSUTF8StringEncoding] > 256) { [owner fail]; return; }
        NSArray<NSString *> *words = [[line stringByTrimmingCharactersInSet:NSCharacterSet.whitespaceAndNewlineCharacterSet]
            componentsSeparatedByString:@" "];
        if (words.count != 3 || ![words[0] isEqualToString:owner.nonce]) { [owner fail]; return; }
        unsigned long long next = 0;
        NSScanner *scanner = [NSScanner scannerWithString:words[1]];
        if (![scanner scanUnsignedLongLong:&next] || !scanner.isAtEnd || next >= owner.steps.count) { [owner fail]; return; }
        NSString *step = owner.steps[next];
        NSString *expected = [NSString stringWithFormat:@"%@ %llu %@\n", owner.nonce, next, step];
        if (![line isEqualToString:expected]) { [owner fail]; return; }
        if (next == owner.sequence) return; // One exact persistent command, not a retry.
        if (next != owner.sequence + 1) { [owner fail]; return; }
        owner.sequence = (uint32_t)next;
        if ([step isEqualToString:@"exit"]) {
            // Normal owned-process termination, whether or not a window is open.
            // Process loss is proven by the supervisor reaping this exact child.
            if (![owner acknowledge:@"exit"]) { [owner fail]; return; }
            [NSApp terminate:nil];
            return;
        }
        if ([step isEqualToString:@"close-window"]) {
            [owner.window orderOut:nil];
            [owner.window close];
            // Both owners must go: a retained view keeps the WindowServer target alive.
            owner.window = nil;
            owner.view = nil;
            if (![owner acknowledge:@"close-window"]) [owner fail];
            return;
        }
        if ([step isEqualToString:@"open-window"]) {
            if (owner.window != nil || owner.view != nil) { [owner fail]; return; }
            [owner createWindow];
        }
        if (owner.window == nil || owner.view == nil) { [owner fail]; return; }
        owner.view.showImage = [step isEqualToString:@"show"] || [step isEqualToString:@"pulse"]
            || [step isEqualToString:@"resize-show"];
        if ([step isEqualToString:@"resize-show"] || [step isEqualToString:@"shrink-blank"]) {
            NSSize size = [step isEqualToString:@"resize-show"] ? NSMakeSize(520, 300) : NSMakeSize(240, 140);
            [owner.window setContentSize:size];
            [owner.view setFrameSize:size];
        }
        if (![owner render:step]) [owner fail];
    }];
}
- (BOOL)applicationShouldTerminateAfterLastWindowClosed:(NSApplication *)sender {
    (void)sender;
    return NO; // Keep the exact process alive while the consumer observes loss.
}
@end

int main(int argc, const char *argv[]) {
#if !defined(__arm64__)
    return 2;
#else
    @autoreleasepool {
        if (argc != 5 || strlen(argv[2]) != 16 || strspn(argv[2], "0123456789abcdef") != 16) return 2;
        NSString *nonce = [NSString stringWithUTF8String:argv[2]];
        NSScanner *scanner = [NSScanner scannerWithString:nonce];
        unsigned long long token = 0;
        if (nonce.length != 16 || ![scanner scanHexLongLong:&token] || !scanner.isAtEnd) return 2;
        // Generation 1 carries the full lifecycle sequence; generation 2 is the
        // separately supervised successor process with its own nonce and title.
        NSArray<NSString *> *steps = nil;
        if (strcmp(argv[4], "1") == 0) {
            steps = @[@"blank", @"arm-blank", @"show", @"pulse", @"blank", @"show", @"resize-show", @"pulse",
                @"blank", @"shrink-blank", @"close-window", @"open-window", @"show", @"pulse", @"blank", @"exit"];
        } else if (strcmp(argv[4], "2") == 0) {
            steps = @[@"blank", @"arm-blank", @"show", @"pulse", @"exit"];
        } else {
            return 2;
        }
        NSURL *imageURL = [NSURL fileURLWithPath:[NSString stringWithUTF8String:argv[3]]];
        NSBitmapImageRep *rep = [NSBitmapImageRep imageRepWithData:[NSData dataWithContentsOfURL:imageURL]];
        if (!rep || rep.pixelsWide != 960 || rep.pixelsHigh != 540) return 2;
        rep.size = NSMakeSize(480, 270); // One source pixel per 2x backing pixel.
        NSImage *image = [[NSImage alloc] initWithSize:NSMakeSize(480, 270)];
        [image addRepresentation:rep];
        [NSApplication sharedApplication];
        if (NSApp.activationPolicy != NSApplicationActivationPolicyAccessory) {
            [NSApp setActivationPolicy:NSApplicationActivationPolicyAccessory];
        }
        if (NSApp.activationPolicy != NSApplicationActivationPolicyAccessory) return 2;
        NSScreen *screen = NSScreen.mainScreen;
        if (!screen || screen.backingScaleFactor != 2.0 || screen.visibleFrame.size.width < 600 || screen.visibleFrame.size.height < 380) return 2;
        Fixture *owner = [Fixture new];
        owner.root = [NSURL fileURLWithPath:[NSString stringWithUTF8String:argv[1]] isDirectory:YES];
        owner.nonce = nonce;
        owner.token = token;
        owner.image = image;
        owner.screen = screen;
        owner.steps = steps;
        owner.originalFrontmost = NSWorkspace.sharedWorkspace.frontmostApplication.processIdentifier;
        NSApp.delegate = owner;
        // Last-resort owned-process fuse, independent of command processing.
        dispatch_after(dispatch_time(DISPATCH_TIME_NOW, 210 * NSEC_PER_SEC), dispatch_get_global_queue(QOS_CLASS_UTILITY, 0), ^{ _exit(124); });
        [NSApp run];
        return [NSFileManager.defaultManager fileExistsAtPath:[owner.root URLByAppendingPathComponent:@"fixture-failure"].path] ? 1 : 0;
    }
#endif
}
