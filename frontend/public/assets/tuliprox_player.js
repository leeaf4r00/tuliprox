(function () {
  "use strict";

  // #region debug
  const DEBUG_SESSION_ID = "player-stalls-918e5d";

  // Set both globals before attaching the player to opt in to diagnostics.
  function playerDiagnosticsEnabled() {
    return window.TULIPROX_PLAYER_DEBUG === true &&
      typeof window.TULIPROX_PLAYER_DEBUG_URL === "string" &&
      window.TULIPROX_PLAYER_DEBUG_URL.trim() !== "";
  }

  function playerDebugLog(msg, data) {
    if (!playerDiagnosticsEnabled()) return;
    try {
      const body = JSON.stringify({
        sessionId: DEBUG_SESSION_ID,
        msg: msg,
        data: data || {},
        hypothesisId: "H1,H2,H3,H4"
      });
      const logUrl = window.TULIPROX_PLAYER_DEBUG_URL;
      if (navigator.sendBeacon && navigator.sendBeacon(logUrl, body)) return;
      fetch(logUrl, { method: "POST", body: body, keepalive: true }).catch(function () {});
    } catch (_) {}
  }

  function mediaSnapshot(video) {
    let bufferedAhead = null;
    try {
      const ranges = video.buffered;
      for (let index = 0; index < ranges.length; index += 1) {
        if (ranges.start(index) <= video.currentTime && ranges.end(index) >= video.currentTime) {
          bufferedAhead = Math.round((ranges.end(index) - video.currentTime) * 10) / 10;
          break;
        }
      }
    } catch (_) {}
    const mediaError = video.error;
    return {
      currentTime: Math.round((video.currentTime || 0) * 10) / 10,
      duration: Number.isFinite(video.duration) ? Math.round(video.duration * 10) / 10 : null,
      bufferedAhead: bufferedAhead,
      readyState: video.readyState,
      networkState: video.networkState,
      paused: video.paused,
      mediaErrorCode: mediaError ? mediaError.code : null
    };
  }

  function attachMediaDiagnostics(video, streamType) {
    if (!playerDiagnosticsEnabled()) return {};

    let waitingStartedAt = null;
    let lastSampleAt = 0;
    const handlers = {};
    ["loadstart", "loadedmetadata", "canplay", "playing", "waiting", "stalled", "error", "ended", "abort"].forEach(function (eventName) {
      handlers[eventName] = function () {
        const now = performance.now();
        const data = Object.assign({ streamType: streamType, event: eventName }, mediaSnapshot(video));
        if (eventName === "waiting" || eventName === "stalled") {
          if (waitingStartedAt === null) waitingStartedAt = now;
        }
        if (eventName === "playing" && waitingStartedAt !== null) {
          data.waitDurationMs = Math.round(now - waitingStartedAt);
          waitingStartedAt = null;
        }
        playerDebugLog("Media event", data);
      };
      video.addEventListener(eventName, handlers[eventName]);
    });
    handlers.timeupdate = function () {
      const now = performance.now();
      if (now - lastSampleAt >= 30000) {
        lastSampleAt = now;
        playerDebugLog("Playback sample", Object.assign({ streamType: streamType }, mediaSnapshot(video)));
      }
    };
    video.addEventListener("timeupdate", handlers.timeupdate);
    return handlers;
  }
  // #endregion

  function trackLabel(track, fallback) {
    return (track && (track.name || track.label || track.lang || track.language)) || fallback;
  }

  function playerTracks(video, hls) {
    const qualities = [];
    const audioTracks = [];
    const subtitleTracks = [];

    if (hls && Array.isArray(hls.levels)) {
      qualities.push({ index: -1, label: "Auto" });
      hls.levels.forEach(function (level, index) {
        const resolution = level.height ? String(level.height) + "p" : "Nível " + String(index + 1);
        const bitrate = level.bitrate ? " · " + (level.bitrate / 1000).toFixed(0) + " kbps" : "";
        qualities.push({ index: index, label: resolution + bitrate });
      });
    } else if (video.videoWidth && video.videoHeight) {
      qualities.push({ index: 0, label: String(video.videoHeight) + "p" });
    }

    if (hls && Array.isArray(hls.audioTracks)) {
      hls.audioTracks.forEach(function (track, index) {
        audioTracks.push({ index: index, label: trackLabel(track, "Áudio " + String(index + 1)) });
      });
    } else if (video.audioTracks) {
      Array.from(video.audioTracks).forEach(function (track, index) {
        audioTracks.push({ index: index, label: trackLabel(track, "Áudio " + String(index + 1)) });
      });
    }

    if (hls && Array.isArray(hls.subtitleTracks)) {
      subtitleTracks.push({ index: -1, label: "Off" });
      hls.subtitleTracks.forEach(function (track, index) {
        subtitleTracks.push({ index: index, label: trackLabel(track, "Legenda " + String(index + 1)) });
      });
    } else if (video.textTracks && video.textTracks.length) {
      subtitleTracks.push({ index: -1, label: "Off" });
      Array.from(video.textTracks).forEach(function (track, index) {
        if (track.kind === "subtitles" || track.kind === "captions") {
          subtitleTracks.push({ index: index, label: trackLabel(track, "Legenda " + String(index + 1)) });
        }
      });
    }

    return JSON.stringify({
      qualities: qualities,
      audio_tracks: audioTracks,
      subtitle_tracks: subtitleTracks,
      selected_quality: hls ? hls.currentLevel : 0,
      selected_audio: hls ? hls.audioTrack : 0,
      selected_subtitle: hls ? hls.subtitleTrack : -1,
      source_width: video.videoWidth || 0,
      source_height: video.videoHeight || 0
    });
  }

  function notifyPlayerTracks(video, hls, onTracks) {
    if (typeof onTracks === "function") {
      onTracks(playerTracks(video, hls));
    }
  }

  window.attachTuliproxVideo = function (video, url, isHls, isMpegTs, isLive, onError, onTracks) {
    let metadataHandler = null;
    let hls = null;
    const streamType = isHls ? "hls" : (isMpegTs ? "mpegts" : "native");
    const mediaDebugHandlers = attachMediaDiagnostics(video, streamType);
    playerDebugLog("Attach player", Object.assign({ streamType: streamType, isLive: Boolean(isLive) }, mediaSnapshot(video)));

    if (isHls && window.Hls && window.Hls.isSupported()) {
      hls = new window.Hls();
      hls.on(window.Hls.Events.ERROR, function (_event, data) {
        playerDebugLog("HLS error", {
          fatal: Boolean(data && data.fatal),
          type: data && data.type ? String(data.type) : null,
          details: data && data.details ? String(data.details) : null,
          responseCode: data && data.response && data.response.code ? data.response.code : null,
          fragmentType: data && data.frag && data.frag.type ? String(data.frag.type) : null,
          fragmentNumber: data && data.frag && Number.isFinite(data.frag.sn) ? data.frag.sn : null,
          level: data && Number.isFinite(data.level) ? data.level : null,
          media: mediaSnapshot(video)
        });
        if (data && data.fatal) {
          onError(null);
        }
      });
      const notify = function () { notifyPlayerTracks(video, hls, onTracks); };
      hls.on(window.Hls.Events.MANIFEST_PARSED, notify);
      hls.on(window.Hls.Events.LEVEL_SWITCHED, notify);
      hls.on(window.Hls.Events.AUDIO_TRACKS_UPDATED, notify);
      hls.on(window.Hls.Events.AUDIO_TRACK_SWITCHED, notify);
      hls.on(window.Hls.Events.SUBTITLE_TRACKS_UPDATED, notify);
      hls.on(window.Hls.Events.SUBTITLE_TRACK_SWITCH, notify);
      hls.attachMedia(video);
      hls.loadSource(url);
      video.__tuliproxPlayerHandle = { hls: hls };
      return { hls: hls, onError: null, metadataHandler: null, mediaDebugHandlers: mediaDebugHandlers };
    }

    if (isMpegTs) {
      if (!window.mpegts || !window.mpegts.isSupported()) {
        onError(null);
        return { hls: null, mpegts: null, onError: null, metadataHandler: null };
      }

      const mpegtsPlayer = window.mpegts.createPlayer({
        type: "mpegts",
        isLive: Boolean(isLive),
        url: url
      });
      let liveRetryTimer = null;
      let liveRetryAttempts = 0;
      let liveRetryScheduled = false;
      let liveRecoveryStartedAt = null;
      let liveRecoveryStartTime = 0;
      const maxLiveRetryAttempts = 5;
      const scheduleLiveRetry = function (reason) {
        if (!isLive) return false;
        if (liveRetryScheduled) return true;
        if (liveRetryAttempts >= maxLiveRetryAttempts) {
          playerDebugLog("Live MPEG-TS retry limit reached", {
            reason: reason,
            attempts: liveRetryAttempts,
            media: mediaSnapshot(video)
          });
          onError(null);
          return true;
        }

        liveRetryAttempts += 1;
        liveRetryScheduled = true;
        liveRecoveryStartedAt = null;
        const delayMs = Math.min(1000 * Math.pow(2, liveRetryAttempts - 1), 8000);
        playerDebugLog("Live MPEG-TS reconnect scheduled", {
          reason: reason,
          attempt: liveRetryAttempts,
          delayMs: delayMs,
          media: mediaSnapshot(video)
        });
        onError(-1);
        if (liveRetryTimer !== null) clearTimeout(liveRetryTimer);
        liveRetryTimer = setTimeout(function () {
          liveRetryTimer = null;
          liveRetryScheduled = false;
          try {
            mpegtsPlayer.unload();
            mpegtsPlayer.load();
            const playPromise = mpegtsPlayer.play();
            if (playPromise && typeof playPromise.catch === "function") {
              playPromise.catch(function () { scheduleLiveRetry("play-rejected"); });
            }
          } catch (error) {
            playerDebugLog("Live MPEG-TS reconnect failed", {
              reason: reason,
              message: error && error.message ? String(error.message) : String(error)
            });
            scheduleLiveRetry("reconnect-failed");
          }
        }, delayMs);
        return true;
      };
      const liveEndedHandler = function () { scheduleLiveRetry("ended"); };
      const livePlayingHandler = function () {
        if (liveRetryAttempts > 0 && !liveRetryScheduled) {
          liveRecoveryStartedAt = performance.now();
          liveRecoveryStartTime = video.currentTime;
          playerDebugLog("Live MPEG-TS playback resumed", {
            attempts: liveRetryAttempts,
            media: mediaSnapshot(video)
          });
          onError(-2);
        }
      };
      const liveProgressHandler = function () {
        if (liveRetryAttempts > 0 && liveRecoveryStartedAt !== null &&
            performance.now() - liveRecoveryStartedAt >= 60000 &&
            video.currentTime >= liveRecoveryStartTime + 50) {
          playerDebugLog("Live MPEG-TS playback recovered", {
            attempts: liveRetryAttempts,
            stableDurationMs: Math.round(performance.now() - liveRecoveryStartedAt),
            media: mediaSnapshot(video)
          });
          liveRetryAttempts = 0;
          liveRecoveryStartedAt = null;
        }
      };
      if (isLive) {
        video.addEventListener("ended", liveEndedHandler);
        video.addEventListener("playing", livePlayingHandler);
        video.addEventListener("timeupdate", liveProgressHandler);
      }
      mpegtsPlayer.on(window.mpegts.Events.ERROR, function (type, detail) {
        playerDebugLog("MPEG-TS error", {
          type: type ? String(type) : null,
          detail: detail ? String(detail) : null,
          media: mediaSnapshot(video)
        });
        if (!scheduleLiveRetry("mpegts-error")) onError(null);
      });
      mpegtsPlayer.attachMediaElement(video);
      mpegtsPlayer.load();
      const playPromise = mpegtsPlayer.play();
      if (playPromise && typeof playPromise.catch === "function") {
        playPromise.catch(function (error) {
          playerDebugLog("MPEG-TS play rejected", { name: error && error.name ? String(error.name) : null });
          onError(null);
        });
      }
      video.__tuliproxPlayerHandle = { mpegts: mpegtsPlayer };
      return {
        hls: null,
        mpegts: mpegtsPlayer,
        onError: null,
        metadataHandler: null,
        mediaDebugHandlers: mediaDebugHandlers,
        liveRecoveryCleanup: function () {
          if (liveRetryTimer !== null) clearTimeout(liveRetryTimer);
          video.removeEventListener("ended", liveEndedHandler);
          video.removeEventListener("playing", livePlayingHandler);
          video.removeEventListener("timeupdate", liveProgressHandler);
        }
      };
    }

    let nativeRecoveryRequested = false;
    let nativeStallTimer = null;
    let nativeMpegTsFallbackAttempted = false;
    let nativeMpegTsFallbackFailed = false;
    let nativeMpegTsFallbackPlayer = null;
    let nativeHandle = null;
    const requestNativeRecovery = function (reason) {
      if (nativeRecoveryRequested || isLive || !(video.currentTime > 0)) return false;
      nativeRecoveryRequested = true;
      if (nativeStallTimer !== null) {
        clearTimeout(nativeStallTimer);
        nativeStallTimer = null;
      }
      const resumePosition = video.currentTime;
      playerDebugLog("Native stream recovery requested", {
        reason: reason,
        resumePosition: Math.round(resumePosition * 10) / 10,
        mediaErrorCode: video.error ? video.error.code : null
      });
      onError(resumePosition);
      return true;
    };
    const scheduleNativeStallRecovery = function () {
      if (nativeStallTimer !== null) clearTimeout(nativeStallTimer);
      if (isLive || video.paused || video.ended) return;
      const stalledAt = video.currentTime;
      nativeStallTimer = setTimeout(function () {
        nativeStallTimer = null;
        if (video.paused || video.ended || video.currentTime > stalledAt + 0.25) return;
        if (!requestNativeRecovery("buffer-timeout")) onError(null);
      }, 12000);
    };
    const clearNativeStallRecovery = function () {
      if (nativeStallTimer !== null) {
        clearTimeout(nativeStallTimer);
        nativeStallTimer = null;
      }
    };
    const startNativeMpegTsFallback = function () {
      if (!window.mpegts || !window.mpegts.isSupported()) return false;

      playerDebugLog("Native format unsupported; retrying as MPEG-TS", mediaSnapshot(video));
      clearNativeStallRecovery();
      video.pause();
      video.removeAttribute("src");
      video.load();

      const mpegtsPlayer = window.mpegts.createPlayer({
        type: "mpegts",
        isLive: Boolean(isLive),
        url: url
      });
      nativeMpegTsFallbackPlayer = mpegtsPlayer;
      nativeHandle.mpegts = mpegtsPlayer;
      video.__tuliproxPlayerHandle = nativeHandle;
      mpegtsPlayer.on(window.mpegts.Events.ERROR, function (type, detail) {
        playerDebugLog("MPEG-TS fallback error", {
          type: type ? String(type) : null,
          detail: detail ? String(detail) : null,
          media: mediaSnapshot(video)
        });
        if (!nativeMpegTsFallbackFailed) {
          nativeMpegTsFallbackFailed = true;
          onError(null);
        }
      });
      mpegtsPlayer.attachMediaElement(video);
      mpegtsPlayer.load();
      const playPromise = mpegtsPlayer.play();
      if (playPromise && typeof playPromise.catch === "function") {
        playPromise.catch(function (error) {
          playerDebugLog("MPEG-TS fallback play rejected", {
            name: error && error.name ? String(error.name) : null
          });
          if (!nativeMpegTsFallbackFailed) {
            nativeMpegTsFallbackFailed = true;
            onError(null);
          }
        });
      }
      return true;
    };
    const nativeErrorHandler = function () {
      const errorCode = video.error ? video.error.code : null;
      if (nativeMpegTsFallbackPlayer) return;
      if (errorCode === 4 && !isHls && !nativeMpegTsFallbackAttempted) {
        nativeMpegTsFallbackAttempted = true;
        video.removeEventListener("error", nativeErrorHandler);
        if (startNativeMpegTsFallback()) return;
      }
      if ((errorCode === 2 || errorCode === 3) && requestNativeRecovery("media-error")) return;
      onError(null);
    };
    nativeHandle = {
      hls: null,
      mpegts: null,
      onError: nativeErrorHandler,
      metadataHandler: null,
      mediaDebugHandlers: mediaDebugHandlers,
      nativeStallTimer: function () { return nativeStallTimer; },
      nativeHandlers: {
        waiting: scheduleNativeStallRecovery,
        stalled: scheduleNativeStallRecovery,
        playing: clearNativeStallRecovery
      }
    };
    video.addEventListener("error", nativeErrorHandler);
    video.addEventListener("waiting", scheduleNativeStallRecovery);
    video.addEventListener("stalled", scheduleNativeStallRecovery);
    video.addEventListener("playing", clearNativeStallRecovery);
    metadataHandler = function () { notifyPlayerTracks(video, null, onTracks); };
    nativeHandle.metadataHandler = metadataHandler;
    video.addEventListener("loadedmetadata", metadataHandler);
    video.src = url;
    video.__tuliproxPlayerHandle = {};
    return nativeHandle;
  };

  window.detachTuliproxVideo = function (handle, video) {
    if (handle && handle.hls) {
      handle.hls.destroy();
    }
    if (handle && handle.mpegts) {
      if (handle.liveRecoveryCleanup) handle.liveRecoveryCleanup();
      handle.mpegts.destroy();
    }
    if (handle && handle.onError) {
      video.removeEventListener("error", handle.onError);
    }
    if (handle && handle.metadataHandler) {
      video.removeEventListener("loadedmetadata", handle.metadataHandler);
    }
    if (handle && handle.mediaDebugHandlers) {
      Object.keys(handle.mediaDebugHandlers).forEach(function (eventName) {
        video.removeEventListener(eventName, handle.mediaDebugHandlers[eventName]);
      });
    }
    if (handle && handle.nativeHandlers) {
      Object.keys(handle.nativeHandlers).forEach(function (eventName) {
        video.removeEventListener(eventName, handle.nativeHandlers[eventName]);
      });
    }
    if (handle && typeof handle.nativeStallTimer === "function") {
      const timer = handle.nativeStallTimer();
      if (timer !== null) clearTimeout(timer);
    }
    delete video.__tuliproxPlayerHandle;
    video.pause();
    video.removeAttribute("src");
    video.load();
  };

  window.setTuliproxVideoQuality = function (video, index) {
    const handle = video.__tuliproxPlayerHandle;
    if (handle && handle.hls) {
      handle.hls.currentLevel = Number(index);
    }
  };

  window.setTuliproxVideoAudio = function (video, index) {
    const handle = video.__tuliproxPlayerHandle;
    if (handle && handle.hls) {
      handle.hls.audioTrack = Number(index);
      return;
    }
    if (video.audioTracks && video.audioTracks[index]) {
      Array.from(video.audioTracks).forEach(function (track) { track.enabled = false; });
      video.audioTracks[index].enabled = true;
    }
  };

  window.setTuliproxVideoSubtitle = function (video, index) {
    const handle = video.__tuliproxPlayerHandle;
    if (handle && handle.hls) {
      handle.hls.subtitleTrack = Number(index);
      return;
    }
    if (video.textTracks) {
      Array.from(video.textTracks).forEach(function (track, trackIndex) {
        track.mode = trackIndex === Number(index) ? "showing" : "disabled";
      });
    }
  };
})();
