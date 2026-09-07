/* Exercise the native consumer's actual readiness predicate without capture. */
#define main native_consumer_main
#include "../../examples/c/native-template-watch.c"
#undef main

int main(void)
{
    madopilot_template_query_snapshot_t snapshot;
    madopilot_frame_stamp_t required;
    INIT(snapshot); INIT(required);
    required.stream = 1;
    required.sequence = 5;
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    snapshot.last_frame = required;
    snapshot.completed = 1;
    snapshot.pending_count = 1;
    snapshot.in_flight_count = 1;
    if (!pending_ready(&snapshot, &required, 0)) {
        fputs("completed busy progress was not ready\n", stderr);
        return 1;
    }
    if (pending_ready(&snapshot, &required, 1)) return 2;
    snapshot.completed = 0;
    if (pending_ready(&snapshot, &required, 0)) return 3;
    snapshot.completed = 1;
    snapshot.flags = 0;
    if (pending_ready(&snapshot, &required, 0)) return 4;
    snapshot.flags = MADOPILOT_TEMPLATE_QUERY_HAS_LAST_FRAME;
    required.geometry = 1;
    if (pending_ready(&snapshot, &required, 0)) return 5;
    required.geometry = 0;
    required.sequence = 6;
    if (pending_ready(&snapshot, &required, 0)) return 6;
    /* Authoritative unchanged pixels can advance last_frame without new work. */
    snapshot.last_frame.sequence = required.sequence;
    if (!pending_ready(&snapshot, &required, 0)) return 7;
    return 0;
}
