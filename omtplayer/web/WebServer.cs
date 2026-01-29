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

using System.Text;
using Microsoft.AspNetCore.Builder;
using Microsoft.AspNetCore.Hosting;
using Microsoft.AspNetCore.Http;
using libomtnet;
using Microsoft.Extensions.Hosting;
using Microsoft.Extensions.DependencyInjection;

namespace omtplayer.web
{
    internal class WebServer
    {
        private WebApplication app;
        private RequestDelegate requestDelegate;
        private string source = "None";
        private string audioDevices = "Default";
        private List<string> log = new List<string>();
        private OMTSettings settings;
        private int port = 8080;

        private const int MAX_LOG_LINES = 256;

        private sealed class NoopLifetime : IHostLifetime
        {
            public Task WaitForStartAsync(CancellationToken _) => Task.CompletedTask;
            public Task StopAsync(CancellationToken _) => Task.CompletedTask;
        }
        public WebServer()
        {
            settings = new OMTSettings(Path.Combine(AppContext.BaseDirectory,"settings.xml"));
            source = settings.GetString("Source", "None");
            audioDevices = settings.GetString("AudioDevices", "Default");
            port = settings.GetInteger("WebServerPort", port);
            WebApplicationBuilder builder = WebApplication.CreateSlimBuilder();
            builder.WebHost.UseUrls("http://0.0.0.0:" + port);
            builder.Services.AddSingleton<IHostLifetime, NoopLifetime>();
            app = builder.Build();
            requestDelegate = new RequestDelegate(pageHandler);
            app.MapGet("/", requestDelegate);
            app.MapPost("/", requestDelegate);
            app.Start();
            WriteLog("WebServer.Port: " + port);
        }

        public void StopServer()
        {
            Task t = app.StopAsync();
            t.Wait();
            WriteLog("WebServer.Stopped");
        }

        public void WriteLog(string message)
        {
            lock (log)
            {
                if (log.Count >= MAX_LOG_LINES)
                {
                    log.RemoveAt(0);
                }
                log.Add(message);
            }
        }

        private string GetReverseLog()
        {
            StringBuilder sb = new StringBuilder();

            lock (log)
            {
                for (int i = log.Count - 1; i >= 0; i--)
                {
                    sb.AppendLine(log[i]);
                }
            }
            return sb.ToString();
        }

        public string Source { get { return source; } }
        public string AudioDevices { get { return audioDevices; } }

        public int Port { get { return port; } }

        private void ChangeSource(string newSource)
        {
            if (this.source != newSource)
            {
                this.source = newSource;
                settings.SetString("Source", newSource);
                settings.Save();
                WriteLog("SourceChanged: " + newSource);
            }
        }

        private void ChangeAudioDevices(string newAudioDevices)
        {
            if (this.audioDevices != newAudioDevices)
            {
                this.audioDevices = newAudioDevices;
                settings.SetString("AudioDevices", newAudioDevices);
                settings.Save();
                WriteLog("AudioDevicesChanged: " + newAudioDevices);
            }
        }

        private Task pageHandler(HttpContext ctx)
        {
            switch (ctx.Request.Path)
            {
                case "/":
                    if (ctx.Request.Method == "POST")
                    {
                        if (ctx.Request.Form.ContainsKey("cmdChange"))
                        {
                            string? s = ctx.Request.Form["cmdChange"];
                            if (s != null)
                            {
                                if (ctx.Request.Form.ContainsKey("cboSource"))
                                {
                                    s = ctx.Request.Form["cboSource"];
                                    if (s != null)
                                    {
                                        ChangeSource(s);                                       
                                    }
                                }
                                
                                List<string> selectedAudio = new List<string>();
                                foreach(var key in ctx.Request.Form.Keys)
                                {
                                    if (key.StartsWith("chkAudio_"))
                                    {
                                        selectedAudio.Add(key.Substring(9)); // Remove chkAudio_ prefix
                                    }
                                }
                                if (selectedAudio.Count > 0)
                                {
                                    ChangeAudioDevices(string.Join(",", selectedAudio));
                                }
                                else
                                {
                                    ChangeAudioDevices("");
                                }
                            }
                        }

                    }
                    ctx.Response.ContentType = "text/html; charset=utf-8";
                    OMTDiscovery discovery = OMTDiscovery.GetInstance();
                    string[] addresses = discovery.GetAddresses();
                    StringBuilder b = new StringBuilder();
                    b.AppendLine(@"<option value=""" + source + @""" selected>" + source + @"</option>");
                    foreach (string address in addresses)
                    {
                        if (address != source)
                        {
                            b.AppendLine(@"<option value=""" + address + @""">" + address + @"</option>");
                        }
                    }

                    // Audio Devices
                    Dictionary<string, string> availableAudio = AudioPlayer.GetAvailableDevices();
                    StringBuilder audioBuilder = new StringBuilder();
                    List<string> currentAudio = new List<string>(audioDevices.Split(','));
                    
                    foreach(var kvp in availableAudio)
                    {
                        string name = kvp.Key;
                        // Determine if checked
                        string checkState = "";
                        if (currentAudio.Contains(name))
                        {
                            checkState = "checked";
                        }
                        
                        audioBuilder.AppendLine($"<input type='checkbox' id='chkAudio_{name}' name='chkAudio_{name}' value='true' {checkState}>");
                        audioBuilder.AppendLine($"<label for='chkAudio_{name}'>{name}</label><br/>");
                    }

                    string html = Properties.Resources.index;
                    html = html.Replace("#SOURCES#", b.ToString());
                    html = html.Replace("#AUDIO_DEVICES#", audioBuilder.ToString());
                    html = html.Replace("#LOG#", GetReverseLog());

                    return ctx.Response.WriteAsync(html);
                default:
                    ctx.Response.StatusCode = 404;
                    return ctx.Response.WriteAsync("");
            }

        }
    }
}
