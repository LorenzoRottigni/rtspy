# RTSPY

Dummy with fakesink

```bash
gst-launch-1.0 -v rtspsrc location=rtsp://127.0.0.1:8554//live name=src \
    src. ! rtph264depay ! h264parse ! avdec_h264 ! videoconvert ! vp8enc ! rtpvp8pay ! application/x-rtp,media=video,payload=96 ! fakesink
```

webrtc pipeline termination:

```bash
gst-launch-1.0 -v rtspsrc location=rtsp://127.0.0.1:8554//live name=src \
    src. ! rtph264depay ! h264parse ! rtph264pay ! application/x-rtp,media=video,payload=96 \
    ! udpsink host=127.0.0.1 port=5004
```
