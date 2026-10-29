use super::*;

pub(super) fn steady_nonmatch(evidence: &mut Evidence) {
    let rig = Rig::new(backend());
    let session = rig.open();
    let query = rig.query(&session, 1, ChangeDetectionPolicy::ExactRgba);
    evidence.checkpoint("before-publication", &rig, 0);
    for index in 0..=20 {
        if index != 0 {
            rig.clock.advance(INTERVAL);
        }
        let started = Instant::now();
        let stamp = rig.publish(&session, 0);
        let observed = settled(&query, stamp);
        assert_eq!(observed.confirmed_observations(), 0, "nonmatch confirmed");
        assert_eq!(
            observed.work().get(OcrWork::Completed) + observed.work().get(OcrWork::SkippedChange),
            index + 1,
            "source was not considered exactly once"
        );
        assert_eq!(
            observed.work().get(OcrWork::Completed),
            1,
            "unchanged negative ran repeated inference"
        );
        let calls = rig.ocr.inner.recognition_count();
        for _ in 0..64 {
            assert!(matches!(query.poll(), OcrTextQueryOutcome::Pending(_)));
        }
        assert_eq!(
            rig.ocr.inner.recognition_count(),
            calls,
            "polling admitted OCR"
        );
        evidence.source("nonmatch-accepted-or-skipped", stamp);
        evidence.endpoint("publication-to-observation", started);
    }
    evidence.checkpoint("steady-nonmatch", &rig, 0);
    let started = Instant::now();
    assert!(matches!(
        query.cancel().as_ref(),
        OcrTextTerminalOutcome::Cancelled
    ));
    evidence.endpoint("query-cancel", started);
    rig.quiescent();
    session.close(&bounded()).expect("session closed");
    evidence.collect(&rig, &[&query], 21);
    evidence.checkpoint("physical-zero", &rig, 0);
}

pub(super) fn positive_consecutive(evidence: &mut Evidence) {
    let rig = Rig::new(
        backend()
            .with_calls(vec![ScriptedOcrCall::new(Vec::new())])
            .with_candidates(positive()),
    );
    let session = rig.open();
    let query = rig.query(&session, 3, ChangeDetectionPolicy::ExactRgba);
    let blank = rig.publish(&session, 0);
    assert_eq!(settled(&query, blank).confirmed_observations(), 0);
    evidence.source("blank-checkpoint", blank);
    evidence.checkpoint("negative-acknowledged", &rig, 0);
    let started = Instant::now();
    let mut confirmations = Vec::with_capacity(3);
    for count in 1..=3 {
        rig.clock.advance(INTERVAL);
        let stamp = rig.publish(&session, 127);
        confirmations.push(stamp);
        if count < 3 {
            let observed = settled(&query, stamp);
            assert_eq!(observed.last_accepted_frame(), Some(stamp));
            assert_eq!(observed.confirmed_observations(), count);
        }
        evidence.source("positive-publication", stamp);
    }
    let terminal = query
        .wait(&bounded())
        .expect("third confirmation completed");
    evidence.endpoint("first-positive-publication-to-terminal", started);
    verify_result(matched(&terminal), confirmations[2], confirmations[0], 3);
    for stamp in &confirmations {
        evidence.source("confirmed-source", *stamp);
    }
    assert!(
        confirmations
            .windows(2)
            .all(|pair| pair[0].sequence() < pair[1].sequence()),
        "confirmation sources repeated"
    );
    assert_eq!(
        query.progress().work().get(OcrWork::SkippedChange),
        0,
        "required confirmation was skipped"
    );
    assert_eq!(
        rig.ocr.inner.recognition_count(),
        4,
        "blank plus three confirmations expected"
    );
    rig.quiescent();
    evidence.observe_retained(std::slice::from_ref(&terminal));
    evidence.checkpoint("matched-retained", &rig, 0);
    session.close(&bounded()).expect("session closed");
    evidence.collect(&rig, &[&query], 4);
    drop(query);
    drop(session);
    drop(rig);
    verify_result(matched(&terminal), confirmations[2], confirmations[0], 3);
    verify_pixels(evidence, matched(&terminal).frame(), confirmations[2], 127);
    println!(
        "# ocr-retention stage=consecutive-parents-dropped resident={:?}",
        evidence.resident()
    );
}

pub(super) fn saturation(evidence: &mut Evidence) {
    let gate = Arc::new(CompletionGate::new());
    let rig = Rig::new(
        backend()
            .with_candidates(positive())
            .with_completion_gate(Arc::clone(&gate)),
    );
    let session = rig.open();
    let queries: Vec<_> = (0..64)
        .map(|_| rig.query(&session, 1, ChangeDetectionPolicy::AnalysisAlways))
        .collect();
    let _release = gate.release_guard();
    evidence.checkpoint("all-query-reservations", &rig, 0);
    let first = rig.publish(&session, 1);
    assert!(gate.wait_until_entered(WAIT), "first OCR did not enter");
    until("all first sources considered", || {
        queries
            .iter()
            .all(|q| q.progress().last_frame() == Some(first))
    });
    let held = queries
        .iter()
        .position(|q| q.progress().physical_in_flight_count() == 1)
        .expect("one held query");
    assert_eq!(
        queries
            .iter()
            .filter(|q| q.progress().physical_in_flight_count() == 1)
            .count(),
        1
    );
    rig.clock.advance(INTERVAL);
    let original_eligible = rig.clock.elapsed() + CAPTURE_TICK;
    for index in 0..=20 {
        if index != 0 {
            rig.clock.advance(Duration::from_secs(1));
        }
        let stamp = rig.publish(&session, 2 + index);
        until("latest pending replacement acknowledged", || {
            queries.iter().all(|q| {
                let p = q.progress();
                p.last_frame() == Some(stamp) && p.pending_count() == 1
            })
        });
        if index == 20 {
            evidence.source("last-replacement", stamp);
        }
    }
    let last_replacement = rig.clock.elapsed();
    assert_eq!(
        rig.ocr.inner.recognition_count(),
        1,
        "held lease admitted another backend call"
    );
    assert!(
        queries[held].progress().work().get(OcrWork::Superseded) >= 20,
        "held query lost its own replacement obligation"
    );
    evidence.checkpoint("held-with-newer-own-obligation", &rig, 0);
    let started = Instant::now();
    expire_original_obligation(&rig.clock, original_eligible, last_replacement);
    let terminals: Vec<_> = queries
        .iter()
        .map(|q| q.wait(&bounded()).expect("eligible queue expired"))
        .collect();
    evidence.endpoint("eligible-expiry-advance-to-terminals", started);
    for (query, terminal) in queries.iter().zip(&terminals) {
        assert!(
            matches!(
                terminal.as_ref(),
                OcrTextTerminalOutcome::Overloaded(OcrTextOverload::QueueExpired)
            ),
            "queue expiry did not win"
        );
        assert_eq!(query.progress().work().get(OcrWork::QueueExpired), 1);
        assert!(
            Arc::ptr_eq(terminal, &query.cancel()),
            "terminal authority changed"
        );
    }
    assert_eq!(rig.engine.ocr_text_observation().physical_ocr_in_flight, 1);
    evidence.checkpoint("logical-overload-physical-held", &rig, 0);
    session.close(&bounded()).expect("logical session close");
    let started = Instant::now();
    gate.release();
    assert!(
        gate.wait_until_completed(WAIT),
        "held backend did not return"
    );
    rig.quiescent();
    evidence.endpoint("held-release-to-physical-zero", started);
    for (query, terminal) in queries.iter().zip(&terminals) {
        assert!(Arc::ptr_eq(
            terminal,
            &query.wait(&bounded()).expect("same terminal")
        ));
        assert_eq!(
            query.progress().confirmed_observations(),
            0,
            "late positive confirmed"
        );
    }
    evidence.work.stale_discards += 1; // The one held backend returned after its generation lost authority.
    evidence.collect(&rig, &queries.iter().collect::<Vec<_>>(), 22);
    evidence.checkpoint("physical-zero", &rig, 0);
}

fn expire_original_obligation(clock: &ManualClock, original: Duration, replacement: Duration) {
    clock.advance(Duration::from_secs(11));
    let now = clock.elapsed();
    let preserved_age = now - original;
    let reset_age = now - replacement;
    assert!(
        preserved_age > Duration::from_secs(30) && reset_age < Duration::from_secs(30),
        "expiry schedule cannot distinguish preserved and reset eligible age"
    );
    println!(
        "# ocr-expiry preserved_eligible_age_ns={} latest_replacement_age_ns={}",
        preserved_age.as_nanos(),
        reset_age.as_nanos()
    );
}

pub(super) fn mixed(evidence: &mut Evidence) {
    mixed_inference(evidence);
    evidence.last_record = 0; // The next engine owns an independent diagnostic sequence.
    mixed_class_return(evidence);
    evidence.last_record = 0;
    mixed_mapping(evidence);
}

fn mixed_inference(evidence: &mut Evidence) {
    const TURNS: usize = 8;
    let gates: Vec<_> = (0..TURNS)
        .map(|_| Arc::new(CompletionGate::new()))
        .collect();
    let rig = Rig::new(
        backend().with_calls(
            gates
                .iter()
                .map(|gate| ScriptedOcrCall::new(Vec::new()).with_completion_gate(Arc::clone(gate)))
                .collect(),
        ),
    );
    let first = rig.open();
    let second = rig.open();
    let queries = [
        rig.query(&first, 1, ChangeDetectionPolicy::AnalysisAlways),
        rig.query(&second, 1, ChangeDetectionPolicy::AnalysisAlways),
    ];
    let templates = [
        rig.template(&first, "ocr-mixed-a"),
        rig.template(&second, "ocr-mixed-b"),
    ];
    let _releases: Vec<_> = gates.iter().map(CompletionGate::release_guard).collect();
    rig.publish(&first, 1);
    assert!(gates[0].wait_until_entered(WAIT), "first mixed OCR entered");
    template_completed(&templates, 1);
    assert_eq!(
        rig.matcher.find_count(),
        2,
        "templates did not progress during held inference"
    );
    let source = first
        .acquire_frame(&FrameRequest::latest(), &bounded())
        .expect("one-shot current source");
    let descriptor = rig.ocr.descriptor();
    let operation = bounded();
    let refusal = first
        .recognize(OcrRequest::new(
            &source,
            descriptor.backend_identity(),
            descriptor.model_identity(),
            OcrRegion::FullFrame,
            CoordinateSpace::CapturePixels,
            &operation,
        ))
        .expect_err("controlled slot must refuse one-shot contention");
    assert_eq!(
        refusal.status(),
        Status::LimitExceeded,
        "controlled Busy status changed"
    );
    assert_eq!(rig.ocr.input.lock().expect("input observations").busy, 1);
    println!(
        "# ocr-oracle workload=mixed-two-session one_shot_busy=controlled-status-only real_onnx_busy=unexecuted"
    );
    evidence.checkpoint("templates-progress-while-ocr-held", &rig, SOURCE_BYTES);
    let mut previous = None;
    for (turn, gate) in gates.iter().enumerate() {
        assert!(
            gate.wait_until_entered(WAIT),
            "bounded OCR turn not entered"
        );
        let source = rig.ocr.input.lock().expect("input observations").sources[turn];
        let owner = if source.stream() == first.stream() {
            0
        } else {
            assert_eq!(
                source.stream(),
                second.stream(),
                "foreign session claimed OCR"
            );
            1
        };
        if let Some(previous) = previous {
            assert_ne!(owner, previous, "ready OCR exceeded S*Q=2 turn bound");
        }
        previous = Some(owner);
        println!(
            "# ocr-fairness workload=mixed-two-session ordinal={} session_index={owner} ready_sessions=2 ready_queries_per_session=1",
            turn + 1
        );
        evidence.source("ocr-claim", source);
        rig.clock.advance(INTERVAL);
        let stamp = rig.publish(&first, 10 + u8::try_from(turn).expect("bounded turn"));
        let other = second
            .acquire_frame(&FrameRequest::latest(), &bounded())
            .expect("second publication")
            .stamp();
        until("both OCR candidates continuously ready", || {
            queries.iter().zip([stamp, other]).all(|(q, stamp)| {
                let p = q.progress();
                p.last_frame() == Some(stamp) && p.pending_count() == 1
            })
        });
        // Every publication has a template turn before the next OCR gate opens.
        template_completed(&templates, (turn + 2) as u64);
        if turn + 1 == TURNS {
            for query in &queries {
                assert!(matches!(
                    query.cancel().as_ref(),
                    OcrTextTerminalOutcome::Cancelled
                ));
            }
            for template in &templates {
                assert!(matches!(
                    template.cancel().as_ref(),
                    TemplateTerminalOutcome::Cancelled
                ));
            }
        }
        gate.release();
    }
    rig.quiescent();
    evidence.work.stale_discards += 1; // Only the final gated turn was cancelled before return.
    first.close(&bounded()).expect("first session closed");
    second.close(&bounded()).expect("second session closed");
    evidence.collect(&rig, &[&queries[0], &queries[1]], 18);
    evidence.checkpoint("mixed-inference-physical-zero", &rig, SOURCE_BYTES);
}

fn template_completed(queries: &[TemplateQuery], count: u64) {
    until("template class progress", || {
        queries.iter().all(|query| {
            let TemplateQueryOutcome::Pending(p) = query.poll() else {
                panic!("template terminated before mixed checkpoint")
            };
            p.work().get(TemplateWork::Completed) >= count && !p.is_in_flight() && !p.is_pending()
        })
    });
}

fn mixed_class_return(evidence: &mut Evidence) {
    const ROUNDS: usize = 3;
    let held_template = Arc::new(CompletionGate::new());
    let ocr_gates: Vec<_> = (0..ROUNDS)
        .map(|_| Arc::new(CompletionGate::new()))
        .collect();
    let template_gates: Vec<_> = (0..ROUNDS)
        .map(|_| Arc::new(CompletionGate::new()))
        .collect();
    let mut template_calls =
        vec![ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(&held_template))];
    template_calls.extend(
        template_gates
            .iter()
            .map(|gate| ScriptedMatchCall::new(Vec::new()).with_completion_gate(Arc::clone(gate))),
    );
    let rig = Rig::with_matcher(
        backend().with_calls(
            ocr_gates
                .iter()
                .map(|gate| ScriptedOcrCall::new(Vec::new()).with_completion_gate(Arc::clone(gate)))
                .collect(),
        ),
        ControlledMatcher::new(FORMAT).with_calls(template_calls),
    );
    let first = rig.open();
    let second = rig.open();
    // Register the complete shared-session query sets before any publication.
    // An unadmitted template can hold that session's acquisition fence.
    let templates = [
        rig.template(&first, "ocr-return-template-a"),
        rig.template(&second, "ocr-return-template-b"),
    ];
    let queries = [
        rig.query(&first, 1, ChangeDetectionPolicy::AnalysisAlways),
        rig.query(&second, 1, ChangeDetectionPolicy::AnalysisAlways),
    ];
    let _held_release = held_template.release_guard();
    let _ocr_releases: Vec<_> = ocr_gates
        .iter()
        .map(CompletionGate::release_guard)
        .collect();
    let _template_releases: Vec<_> = template_gates
        .iter()
        .map(CompletionGate::release_guard)
        .collect();
    let mut stamps = [
        rig.publish(&first, 1),
        second
            .acquire_frame(&FrameRequest::latest(), &bounded())
            .expect("second initial source")
            .stamp(),
    ];
    assert!(
        held_template.wait_until_entered(WAIT),
        "one template must occupy the other worker"
    );
    assert!(
        ocr_gates[0].wait_until_entered(WAIT),
        "initial OCR turn did not enter"
    );
    let held_index = templates
        .iter()
        .position(
            |query| matches!(query.poll(), TemplateQueryOutcome::Pending(p) if p.is_in_flight()),
        )
        .expect("one template occupies the other worker");
    let ready_index = 1 - held_index;
    for (round, (ocr_gate, template_gate)) in ocr_gates.iter().zip(&template_gates).enumerate() {
        assert!(
            ocr_gate.wait_until_entered(WAIT),
            "next OCR turn did not enter"
        );
        if round != 0 {
            // This template has now completed the preceding source. Only then
            // can its session's all-query fence acquire the next publication.
            let TemplateQueryOutcome::Pending(p) = templates[ready_index].poll() else {
                panic!("template terminated before the next class-return round");
            };
            assert_eq!(
                p.last_frame(),
                Some(stamps[ready_index]),
                "template completed a different source"
            );
            rig.clock.advance(INTERVAL);
            stamps = [
                rig.publish(&first, 40 + u8::try_from(round).expect("bounded round")),
                second
                    .acquire_frame(&FrameRequest::latest(), &bounded())
                    .expect("second latest")
                    .stamp(),
            ];
        }
        // Template progress.last_frame is a completed/considered analysis, not
        // the pending source. Publication enqueues each template before its
        // same-session OCR query, whose last_frame acknowledges this source.
        until("both classes ready before OCR physical return", || {
            queries
                .iter()
                .zip(stamps)
                .all(|(query, frame)| query.progress().last_frame() == Some(frame))
                && queries.iter().any(|query| {
                    let p = query.progress();
                    p.pending_count() == 1 && p.physical_in_flight_count() == 0
                })
                && matches!(templates[ready_index].poll(), TemplateQueryOutcome::Pending(p)
                    if p.is_pending() && !p.is_in_flight()
                        && p.work().get(TemplateWork::Completed) == round as u64)
        });
        let template_before = rig.matcher.find_count();
        let ocr_before = rig.ocr.inner.recognition_count();
        assert_eq!(template_before, round + 1);
        assert_eq!(ocr_before, round + 1);
        evidence.checkpoint("both-classes-ready-other-worker-held", &rig, 0);
        evidence.source("class-return-ready-template", stamps[ready_index]);
        // Template A occupies one worker throughout. Releasing OCR frees the
        // only dispatching worker, so the next backend admission cannot be an
        // inference-held bypass on another worker. Either next call then gates.
        ocr_gate.release();
        until("next actual cross-class admission", || {
            rig.matcher.find_count() != template_before
                || rig.ocr.inner.recognition_count() != ocr_before
        });
        assert_eq!(
            rig.matcher.find_count(),
            template_before + 1,
            "ready template lost its return-boundary turn"
        );
        assert_eq!(
            rig.ocr.inner.recognition_count(),
            ocr_before,
            "OCR reclaimed the next template turn"
        );
        assert!(
            template_gate.wait_until_entered(WAIT),
            "selected template backend did not enter"
        );
        assert_eq!(rig.engine.ocr_text_observation().physical_ocr_in_flight, 0);
        println!(
            "# ocr-class-return round={} next_admission=template template_backend_calls={} ocr_backend_calls={}",
            round + 1,
            rig.matcher.find_count(),
            rig.ocr.inner.recognition_count()
        );
        if round + 1 != ROUNDS {
            template_gate.release();
        }
    }
    for query in &queries {
        assert!(matches!(
            query.cancel().as_ref(),
            OcrTextTerminalOutcome::Cancelled
        ));
    }
    for template in &templates {
        assert!(matches!(
            template.cancel().as_ref(),
            TemplateTerminalOutcome::Cancelled
        ));
    }
    template_gates[ROUNDS - 1].release();
    held_template.release();
    assert!(template_gates[ROUNDS - 1].wait_until_completed(WAIT));
    assert!(held_template.wait_until_completed(WAIT));
    until("controlled template backend returns", || {
        rig.matcher.completion_count() == ROUNDS + 1
    });
    rig.quiescent();
    first
        .close(&bounded())
        .expect("first class-return session closed");
    second
        .close(&bounded())
        .expect("second class-return session closed");
    evidence.collect(&rig, &[&queries[0], &queries[1]], 2 * ROUNDS as u64);
    evidence.checkpoint("class-return-physical-zero", &rig, 0);
}

fn mixed_mapping(evidence: &mut Evidence) {
    let gate = Arc::new(CompletionGate::new());
    let producer =
        ControlledProducer::new(EXTENT, FORMAT, 2, 32).expect("bounded detached producer");
    producer.set_conversion_gate(Some(Arc::clone(&gate)));
    let rig = Rig::new(backend().with_candidates(positive()));
    let first = rig.open();
    let second = rig.open();
    let held = rig.query(&first, 1, ChangeDetectionPolicy::AnalysisAlways);
    let _release = gate.release_guard();
    rig.capture
        .publish_from(&producer, 1)
        .expect("opaque publication");
    assert!(gate.wait_until_entered(WAIT), "OCR mapping did not enter");
    let other = rig.query(&second, 1, ChangeDetectionPolicy::AnalysisAlways);
    let templates = [
        rig.template(&first, "ocr-barrier-a"),
        rig.template(&second, "ocr-barrier-b"),
    ];
    let original_eligible = rig.clock.elapsed() + CAPTURE_TICK;
    let mut previous_superseded = [None; 2];
    for index in 0..=20 {
        if index != 0 {
            rig.clock.advance(Duration::from_secs(1));
        }
        rig.clock.advance(CAPTURE_TICK);
        rig.capture
            .publish_from(&producer, 2 + index)
            .expect("opaque replacement");
        let stamps = [
            first
                .acquire_frame(&FrameRequest::latest(), &bounded())
                .expect("first latest")
                .stamp(),
            second
                .acquire_frame(&FrameRequest::latest(), &bounded())
                .expect("second latest")
                .stamp(),
        ];
        until("barrier-deferred acquisition advances", || {
            [&held, &other].iter().zip(stamps).all(|(query, stamp)| {
                let p = query.progress();
                p.last_frame() == Some(stamp) && p.pending_count() == 1
            }) && templates
                .iter()
                .zip(previous_superseded)
                .all(|(query, previous)| {
                    let TemplateQueryOutcome::Pending(p) = query.poll() else {
                        panic!("template terminated before queue expiry")
                    };
                    p.is_pending()
                        && !p.is_in_flight()
                        && previous
                            .is_none_or(|count| p.work().get(TemplateWork::Superseded) == count + 1)
                })
        });
        for (previous, query) in previous_superseded.iter_mut().zip(&templates) {
            let TemplateQueryOutcome::Pending(p) = query.poll() else {
                panic!("template terminated before queue expiry");
            };
            *previous = Some(p.work().get(TemplateWork::Superseded));
        }
    }
    let last_replacement = rig.clock.elapsed();
    assert_eq!(
        producer.conversion_attempts(),
        1,
        "template entered held shared-device conversion"
    );
    assert_eq!(
        rig.matcher.find_count(),
        0,
        "template mapped through an active barrier"
    );
    assert_eq!(
        rig.ocr.inner.recognition_count(),
        0,
        "held conversion reached inference"
    );
    evidence.checkpoint("shared-device-mapping-barrier", &rig, 0);
    expire_original_obligation(&rig.clock, original_eligible, last_replacement);
    for query in [&held, &other] {
        assert!(matches!(
            query.wait(&bounded()).expect("OCR queue expiry").as_ref(),
            OcrTextTerminalOutcome::Overloaded(OcrTextOverload::QueueExpired)
        ));
    }
    for query in &templates {
        assert!(matches!(
            query
                .wait(&bounded())
                .expect("template queue expiry")
                .as_ref(),
            TemplateTerminalOutcome::Overloaded(TemplateOverload::QueueExpired)
        ));
    }
    assert_eq!(
        producer.conversion_attempts(),
        1,
        "queue expiry entered held conversion"
    );
    first.close(&bounded()).expect("first logical close");
    second.close(&bounded()).expect("second logical close");
    evidence.checkpoint("mapping-expiry-logical-close", &rig, 0);
    let started = Instant::now();
    gate.release();
    assert!(gate.wait_until_completed(WAIT));
    rig.quiescent();
    evidence.endpoint("mapping-release-to-physical-zero", started);
    evidence.collect(&rig, &[&held, &other], 44);
    evidence.checkpoint("mapping-physical-zero", &rig, 0);
}

pub(super) fn retained(evidence: &mut Evidence) {
    let producer =
        ControlledProducer::new(EXTENT, FORMAT, 2, 32).expect("bounded detached producer");
    let rig = Rig::new(backend().with_candidates(positive()));
    let session = rig.open();
    let queries: Vec<_> = (0..4)
        .map(|_| rig.query(&session, 1, ChangeDetectionPolicy::ExactRgba))
        .collect();
    evidence.checkpoint("before-retain", &rig, 0);
    rig.capture
        .publish_from(&producer, 17)
        .expect("original detached publication");
    let original = session
        .acquire_frame(&FrameRequest::latest(), &bounded())
        .expect("original frame")
        .stamp();
    let results: Vec<_> = queries
        .iter()
        .map(|q| q.wait(&bounded()).expect("retained match"))
        .collect();
    rig.quiescent();
    let frames: Vec<_> = results
        .iter()
        .map(|terminal| matched(terminal).frame().clone())
        .collect();
    let extent: u64 = results
        .iter()
        .map(|terminal| matched(terminal).retained_extent().total_bytes())
        .sum();
    assert_eq!(rig.engine.ocr_text_observation().retained_results, 4);
    assert_eq!(
        rig.engine
            .ocr_text_observation()
            .retained_result_extent_bytes,
        extent
    );
    let same_owner = Arc::clone(&results[0]);
    assert_eq!(
        rig.engine.ocr_text_observation().retained_results,
        4,
        "Arc handle clone counted as a new result"
    );
    drop(same_owner);
    evidence.observe_retained(&results);
    evidence.checkpoint("four-results-four-frame-owners", &rig, 4 * SOURCE_BYTES);
    for index in 0..20 {
        rig.clock.advance(CAPTURE_TICK);
        rig.capture
            .publish_from(&producer, 50 + index)
            .expect("producer progresses with retained frames");
        let stamp = session
            .acquire_frame(&FrameRequest::latest(), &bounded())
            .expect("newer producer frame")
            .stamp();
        assert_eq!(
            stamp.sequence().value(),
            original.sequence().value() + u64::from(index) + 1
        );
        assert_eq!(
            producer.producer_slots_free(),
            producer.pool(),
            "retention pinned a producer slot"
        );
    }
    for terminal in &results {
        verify_result(matched(terminal), original, original, 1);
    }
    session.close(&bounded()).expect("logical parent close");
    evidence.collect(&rig, &queries.iter().collect::<Vec<_>>(), 21);
    evidence.checkpoint("session-closed-results-retained", &rig, 4 * SOURCE_BYTES);
    drop(queries);
    drop(session);
    drop(rig);
    for terminal in &results {
        verify_result(matched(terminal), original, original, 1);
        verify_pixels(evidence, matched(terminal).frame(), original, 17);
    }
    println!(
        "# ocr-retention stage=all-parents-dropped result_owners=4 result_logical_extent_bytes={extent} separate_frame_extent_bytes={} detached_live={} resident={:?}",
        4 * SOURCE_BYTES,
        producer.detached_budget() - producer.detached_slots_free(),
        evidence.resident()
    );
    drop(results);
    for frame in &frames {
        verify_pixels(evidence, frame, original, 17);
    }
    println!(
        "# ocr-retention stage=result-owners-dropped result_owners=0 result_logical_extent_bytes=0 separate_frame_extent_bytes={} detached_live={} resident={:?}",
        4 * SOURCE_BYTES,
        producer.detached_budget() - producer.detached_slots_free(),
        evidence.resident()
    );
    drop(frames);
    until("detached storage release", || {
        producer.detached_slots_free() == producer.detached_budget()
    });
    println!(
        "# ocr-retention stage=separate-frame-owners-dropped result_owners=0 separate_frame_extent_bytes=0 detached_live={} conversions={} resident={:?}",
        producer.detached_budget() - producer.detached_slots_free(),
        producer.conversions(),
        evidence.resident()
    );
}

pub(super) fn cancellation_close(evidence: &mut Evidence) {
    held_close(evidence, true);
    evidence.last_record = 0;
    held_close(evidence, false);
}

fn held_close(evidence: &mut Evidence, mapping: bool) {
    let gate = Arc::new(CompletionGate::new());
    let producer =
        ControlledProducer::new(EXTENT, FORMAT, 2, 8).expect("bounded detached producer");
    if mapping {
        producer.set_conversion_gate(Some(Arc::clone(&gate)));
    }
    let ocr = if mapping {
        backend().with_candidates(positive())
    } else {
        backend()
            .with_candidates(positive())
            .with_completion_gate(Arc::clone(&gate))
    };
    let rig = Rig::new(ocr);
    let session = rig.open();
    let query = rig.query(&session, 1, ChangeDetectionPolicy::AnalysisAlways);
    let _release = gate.release_guard();
    rig.capture
        .publish_from(&producer, 33)
        .expect("held-close publication");
    assert!(gate.wait_until_entered(WAIT), "held work did not enter");
    let template = rig.template(&session, "ocr-close-pending-template");
    if mapping {
        until(
            "template deferred behind mapping",
            || matches!(template.poll(), TemplateQueryOutcome::Pending(p) if p.is_pending()),
        );
        assert_eq!(producer.conversion_attempts(), 1);
    } else {
        template_completed(std::slice::from_ref(&template), 1);
    }
    evidence.checkpoint(
        if mapping {
            "mapping-held"
        } else {
            "inference-held"
        },
        &rig,
        0,
    );
    let cancellation = CancellationToken::new();
    cancellation.cancel();
    let started = Instant::now();
    let error = query
        .wait(&OperationContext::new().with_cancellation(cancellation))
        .expect_err("independent wait cancellation");
    assert_eq!(error.status(), Status::Cancelled);
    assert!(
        matches!(query.poll(), OcrTextQueryOutcome::Pending(_)),
        "wait cancellation mutated the query"
    );
    evidence.endpoint("independent-wait-cancel", started);
    let started = Instant::now();
    let terminal = query.cancel();
    assert!(matches!(
        terminal.as_ref(),
        OcrTextTerminalOutcome::Cancelled
    ));
    evidence.endpoint("query-cancel", started);
    assert_eq!(query.progress().in_flight_count(), 0);
    assert_eq!(query.progress().physical_in_flight_count(), 1);
    let started = Instant::now();
    session
        .close(&bounded())
        .expect("logical close must not join OCR");
    evidence.endpoint("session-close", started);
    assert!(matches!(
        template.wait(&bounded()).expect("template closed").as_ref(),
        TemplateTerminalOutcome::SessionClosed
    ));
    evidence.collect(&rig, &[&query], 1);
    evidence.checkpoint("logical-closed-physical-held", &rig, 0);
    let weak_backend = Arc::downgrade(&rig.ocr);
    let Rig {
        engine,
        capture,
        ocr,
        matcher,
        clock,
        reader,
    } = rig;
    let (finished, completion) = std::sync::mpsc::channel();
    let started = Instant::now();
    let dropper = std::thread::spawn(move || {
        drop((
            query, template, session, engine, capture, ocr, matcher, clock,
        ));
        finished.send(()).expect("drop observer remains alive");
    });
    if completion.recv_timeout(WAIT).is_err() {
        gate.release();
        panic!("last runtime owner drop waited on held OCR");
    }
    dropper.join().expect("parent owner release returned");
    evidence.endpoint("last-runtime-owner-drop", started);
    assert!(
        weak_backend.upgrade().is_some(),
        "physical lease released backend early"
    );
    until("diagnostics sealed before physical return", || {
        evidence.drain(&reader)
    });
    let started = Instant::now();
    gate.release();
    assert!(
        gate.wait_until_completed(WAIT),
        "physical held call did not finish"
    );
    until("actual backend and detached storage retirement", || {
        weak_backend.upgrade().is_none()
            && producer.detached_slots_free() == producer.detached_budget()
    });
    evidence.endpoint("held-release-to-actual-resource-retirement", started);
    assert!(
        matches!(reader.drain(), DiagnosticDrain::EndOfStream),
        "late diagnostics escaped sealed runtime"
    );
    assert!(
        matches!(terminal.as_ref(), OcrTextTerminalOutcome::Cancelled),
        "late completion changed terminal"
    );
    if !mapping {
        evidence.work.stale_discards += 1;
    }
    println!(
        "# ocr-physical-retirement stage={} backend_live=false detached_live=0 diagnostics=sealed resident={:?}",
        if mapping { "mapping" } else { "inference" },
        evidence.resident()
    );
}

pub(super) fn startup(evidence: &mut Evidence) {
    let started = Instant::now();
    let rig = Rig::new(
        backend()
            .with_calls(vec![ScriptedOcrCall::new(Vec::new())])
            .with_candidates(positive()),
    );
    evidence.endpoint("controlled-engine-construction", started);
    let started = Instant::now();
    let session = rig.open();
    evidence.endpoint("controlled-session-open", started);
    let started = Instant::now();
    let query = rig.query(&session, 1, ChangeDetectionPolicy::ExactRgba);
    evidence.endpoint("first-query-start", started);
    let blank = rig.publish(&session, 0);
    settled(&query, blank);
    rig.clock.advance(INTERVAL);
    let started = Instant::now();
    let stamp = rig.publish(&session, 127);
    let terminal = query
        .wait(&bounded())
        .expect("startup transition completed");
    evidence.endpoint("first-positive-to-terminal", started);
    verify_result(matched(&terminal), stamp, stamp, 1);
    rig.quiescent();
    evidence.observe_retained(std::slice::from_ref(&terminal));
    let started = Instant::now();
    session.close(&bounded()).expect("startup session closed");
    evidence.endpoint("logical-close", started);
    evidence.collect(&rig, &[&query], 2);
    evidence.checkpoint("startup-physical-zero", &rig, 0);
    drop((query, terminal, session, rig));
    println!(
        "# ocr-retirement stage=startup-owners-dropped resident={:?}",
        evidence.resident()
    );
}
