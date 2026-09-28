(function () {
  "use strict";

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

    if (isHls && window.Hls && window.Hls.isSupported()) {
      hls = new window.Hls();
      hls.on(window.Hls.Events.ERROR, function (_event, data) {
        if (data && data.fatal) {
          onError();
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
      return { hls: hls, onError: null, metadataHandler: null };
    }

    if (isMpegTs) {
      if (!window.mpegts || !window.mpegts.isSupported()) {
        onError();
        return { hls: null, mpegts: null, onError: null, metadataHandler: null };
      }

      const mpegtsPlayer = window.mpegts.createPlayer({
        type: "mpegts",
        isLive: Boolean(isLive),
        url: url
      });
      mpegtsPlayer.on(window.mpegts.Events.ERROR, function () {
        onError();
      });
      mpegtsPlayer.attachMediaElement(video);
      mpegtsPlayer.load();
      const playPromise = mpegtsPlayer.play();
      if (playPromise && typeof playPromise.catch === "function") {
        playPromise.catch(onError);
      }
      video.__tuliproxPlayerHandle = { mpegts: mpegtsPlayer };
      return { hls: null, mpegts: mpegtsPlayer, onError: null, metadataHandler: null };
    }

    video.addEventListener("error", onError, { once: true });
    metadataHandler = function () { notifyPlayerTracks(video, null, onTracks); };
    video.addEventListener("loadedmetadata", metadataHandler);
    video.src = url;
    video.__tuliproxPlayerHandle = {};
    return { hls: null, onError: onError, metadataHandler: metadataHandler };
  };

  window.detachTuliproxVideo = function (handle, video) {
    if (handle && handle.hls) {
      handle.hls.destroy();
    }
    if (handle && handle.mpegts) {
      handle.mpegts.destroy();
    }
    if (handle && handle.onError) {
      video.removeEventListener("error", handle.onError);
    }
    if (handle && handle.metadataHandler) {
      video.removeEventListener("loadedmetadata", handle.metadataHandler);
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
