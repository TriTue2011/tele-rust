#!/usr/bin/env python3
"""ĐO độ trễ ĐƯỜNG ÂM THANH của gateway: một nguồn PCM thô 16 kHz phát đúng thời gian thực (HTTP chunked) cứ 2 giây chèn một tiếng «tách» có giờ ghi sẵn;
chạy ĐÚNG lệnh ffmpeg của gateway (tham số 1, thay PORT) trong container tạm có ffmpeg, đọc PCM ra pipe ĐÚNG nhịp thời gian thực như NTgCalls,
in độ trễ (ms) của từng tiếng tách. Dùng: do_tre_mic.py "<lệnh ffmpeg, PORT thay cổng>" <cổng> [wav]   (cần docker + ảnh có ffmpeg: biến IMAGE)
Đo 21/11/2026: lệnh KHÔNG có cờ giảm trễ ≈ 3186 ms; có LIVE_IN ≈ 64 ms. Mã nguồn đo này chỉ dùng dữ liệu tổng hợp (không dữ liệu thật)."""
import socket, subprocess, struct, threading, time, sys, statistics, os, random
