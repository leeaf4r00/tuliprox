(function () {
  "use strict";

  window.attachTuliproxVideo = function (video, url, isHls, isMpegTs, isLive, onError) {
    if (isHls && window.Hls && window.Hls.isSupported()) {
      const hls = new window.Hls();
      hls.on(window.Hls.Events.ERROR, function (_event, data) {
        if (data && data.fatal) {
          onError();
        }
      });
      hls.attachMedia(video);
      hls.loadSource(url);
      return { hls: hls, onError: null };
    }

    if (isMpegTs) {
      if (!window.mpegts || !window.mpegts.isSupported()) {
        onError();
        return { hls: null, mpegts: null, onError: null };
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
      return { hls: null, mpegts: mpegtsPlayer, onError: null };
    }

    video.addEventListener("error", onError, { once: true });
    video.src = url;
    return { hls: null, onError: onError };
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
    video.pause();
    video.removeAttribute("src");
    video.load();
  };
})();
