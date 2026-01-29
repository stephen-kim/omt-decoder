/*
* MIT License
*
* Copyright (c) 2025 Open Media Transport Contributors
*
* Permission is hereby granted, free of charge, to any person obtaining a copy
* of this software and associated documentation files (the "Software"), to deal
* in the Software without restriction, including without limitation the rights
* to use, copy, modify, merge, publish, distribute, sublicense, and/or sell
* copies of the Software, and to permit persons to whom the Software is
* furnished to do so, subject to the following conditions:
*
* The above copyright notice and this permission notice shall be included in all
* copies or substantial portions of the Software.
*
* THE SOFTWARE IS PROVIDED "AS IS", WITHOUT WARRANTY OF ANY KIND, EXPRESS OR
* IMPLIED, INCLUDING BUT NOT LIMITED TO THE WARRANTIES OF MERCHANTABILITY,
* FITNESS FOR A PARTICULAR PURPOSE AND NONINFRINGEMENT. IN NO EVENT SHALL THE
* AUTHORS OR COPYRIGHT HOLDERS BE LIABLE FOR ANY CLAIM, DAMAGES OR OTHER
* LIABILITY, WHETHER IN AN ACTION OF CONTRACT, TORT OR OTHERWISE, ARISING FROM,
* OUT OF OR IN CONNECTION WITH THE SOFTWARE OR THE USE OR OTHER DEALINGS IN THE
* SOFTWARE.
*
*/

using System;
using System.Runtime.InteropServices;
using System.Collections.Concurrent;
using System.IO;
using System.Text.RegularExpressions;
using System.Threading;

namespace omtplayer
{
    public class AudioPlayer : IDisposable
    {
        private List<IntPtr> pcmHandles = new List<IntPtr>();
        private int channels;
        private int rate;
        private bool running = true;
        private ConcurrentQueue<float[]> audioQueue = new ConcurrentQueue<float[]>();
        private Thread? playbackThread;
        private List<string> activeDevices = new List<string>();
        private object deviceLock = new object();
        private Action<string>? logAction;


        // ALSA P/Invoke Definitions
        private const string LibAsound = "libasound.so.2";

        [DllImport(LibAsound)]
        private static extern int snd_pcm_open(ref IntPtr pcm, string name, int stream, int mode);

        [DllImport(LibAsound)]
        private static extern int snd_pcm_set_params(IntPtr pcm, int format, int access, int channels, int rate, int soft_resample, int latency);

        [DllImport(LibAsound)]
        private static extern int snd_pcm_writei(IntPtr pcm, IntPtr buffer, ulong size);

        [DllImport(LibAsound)]
        private static extern int snd_pcm_prepare(IntPtr pcm);
        
        [DllImport(LibAsound)]
        private static extern int snd_pcm_recover(IntPtr pcm, int err, int silent);

        [DllImport(LibAsound)]
        private static extern int snd_pcm_close(IntPtr pcm);

        [DllImport(LibAsound)]
        private static extern IntPtr snd_strerror(int errnum);

        private const int SND_PCM_STREAM_PLAYBACK = 0;
        private const int SND_PCM_FORMAT_FLOAT_LE = 14; // SND_PCM_FORMAT_FLOAT_LE
        private const int SND_PCM_ACCESS_RW_INTERLEAVED = 3;

        public AudioPlayer(Action<string>? logger = null)
        {
            this.logAction = logger;
            // Default to first available if none selected, or just "default"
            // For now, let's default to "default" if nothing else.
            // But usually the user might want HDMI (which is default often) or USB.
            // Let's start with "default" in the active list.
            activeDevices.Add("default");
            
            playbackThread = new Thread(PlaybackLoop);
            playbackThread.Start();
        }

        public void SetActiveDevices(List<string> devices)
        {
            lock (deviceLock)
            {
                activeDevices = new List<string>(devices);
                // Force re-open on next enqueue
                CloseAudio();
            }
        }

        public static Dictionary<string, string> GetAvailableDevices()
        {
            Dictionary<string, string> devices = new Dictionary<string, string>();
            devices.Add("Default", "default");
            
            try
            {
                if (File.Exists("/proc/asound/cards"))
                {
                    string[] lines = File.ReadAllLines("/proc/asound/cards");
                    foreach (string line in lines)
                    {
                        // 0 [Headphones     ]: bcm2835_headphon - bcm2835 Headphones
                        Match match = Regex.Match(line, @"^ (\d+) \[.*?\]: (.*?) - (.*)");
                        if (match.Success)
                        {
                            int cardNum = int.Parse(match.Groups[1].Value);
                            string id = match.Groups[2].Value;
                            string name = match.Groups[3].Value;
                            
                            // Use plughw for direct hardware access with format conversion
                            string alsaName = $"plughw:{cardNum},0";
                            string displayName = $"{name} ({id})";
                            
                            if (!devices.ContainsKey(displayName))
                            {
                                devices.Add(displayName, alsaName);
                            }
                        }
                    }
                }
            }
            catch (Exception ex)
            {
                Console.WriteLine($"Error scanning audio devices: {ex.Message}");
            }
            return devices;
        }

        private void Log(string message)
        {
            if (logAction != null) logAction(message);
            Console.WriteLine(message);
        }

        private void CloseAudio()
        {
            foreach (var handle in pcmHandles)
            {
                if (handle != IntPtr.Zero)
                {
                    snd_pcm_close(handle);
                }
            }
            pcmHandles.Clear();
        }

        private void OpenAudio(int channels, int rate)
        {
            CloseAudio();

            lock (deviceLock)
            {
                foreach (string deviceName in activeDevices)
                {
                    IntPtr handle = IntPtr.Zero;
                    int err = snd_pcm_open(ref handle, deviceName, SND_PCM_STREAM_PLAYBACK, 0);
                    if (err < 0)
                    {
                        Log($"Audio open error ({deviceName}): {Marshal.PtrToStringAnsi(snd_strerror(err))}");
                        continue;
                    }

                    err = snd_pcm_set_params(handle, SND_PCM_FORMAT_FLOAT_LE, SND_PCM_ACCESS_RW_INTERLEAVED, channels, rate, 1, 50000);
                    if (err < 0)
                    {
                        Log($"Audio set params error ({deviceName}): {Marshal.PtrToStringAnsi(snd_strerror(err))}");
                        snd_pcm_close(handle);
                        continue;
                    }
                    pcmHandles.Add(handle);
                    Log($"Audio Opened ({deviceName}): {channels}ch {rate}Hz");
                }
            }
            this.channels = channels;
            this.rate = rate;
        }

        public void Enqueue(IntPtr planarData, int channels, int samplesPerChannel, int rate)
        {
            // If config changed or (implicitly) devices changed (handled by SetActiveDevices clearing handles?)
            // Actually SetActiveDevices calls CloseAudio, so pcmHandles will be empty, forcing re-open here.
            
            // Note: concurrency - if SetActiveDevices happens during this check?
            // Lock is safest.
            lock (deviceLock)
            {
                if (channels != this.channels || rate != this.rate || pcmHandles.Count == 0 && activeDevices.Count > 0)
                {
                    OpenAudio(channels, rate);
                }
            }

            if (pcmHandles.Count == 0) return;

            // Planar to Interleaved conversion
            float[] interleaved = new float[channels * samplesPerChannel];
            unsafe
            {
                float* src = (float*)planarData;
                for (int s = 0; s < samplesPerChannel; s++)
                {
                    for (int c = 0; c < channels; c++)
                    {
                        interleaved[s * channels + c] = src[(c * samplesPerChannel) + s];
                    }
                }
            }

            audioQueue.Enqueue(interleaved);
            while (audioQueue.Count > 10)
            {
                audioQueue.TryDequeue(out _);
            }
        }

        private void PlaybackLoop()
        {
            while (running)
            {
                if (audioQueue.TryDequeue(out float[]? buffer))
                {
                    if (buffer != null)
                    {
                        unsafe
                        {
                            fixed (float* p = buffer)
                            {
                                ulong frames = (ulong)(buffer.Length / channels);
                                
                                // Write to all open devices
                                // We iterate a copy or lock? Handles only change on format change or SetActiveDevices.
                                // SetActiveDevices clears list.
                                lock (deviceLock) 
                                {
                                    foreach (var handle in pcmHandles)
                                    {
                                        if (handle != IntPtr.Zero)
                                        {
                                            int err = snd_pcm_writei(handle, (IntPtr)p, frames);
                                            if (err < 0)
                                            {
                                                err = snd_pcm_recover(handle, err, 0);
                                                if (err < 0)
                                                {
                                                    // Log($"ALSA write failed: {Marshal.PtrToStringAnsi(snd_strerror(err))}");
                                                }
                                            }
                                        }
                                    }
                                }
                            }
                        }
                    }
                }
                else
                {
                    Thread.Sleep(1);
                }
            }
        }

        public void Dispose()
        {
            running = false;
            if (playbackThread != null)
            {
                playbackThread.Join(100);
            }
            lock (deviceLock)
            {
                CloseAudio();
            }
        }
    }
}
