# Safe transport-boundary regression: no administrator rights or service changes.
$ErrorActionPreference = 'Stop'
$ProgressPreference = 'SilentlyContinue'
Add-Type -AssemblyName System.Net.Http
Add-Type -ReferencedAssemblies System.Net.Http -TypeDefinition @'
using System;
using System.IO;
using System.Net;
using System.Net.Http;
using System.Net.Http.Headers;
using System.Collections.Generic;
using System.Threading;
using System.Threading.Tasks;
public class BrokenDownload : MemoryStream {
    public BrokenDownload(byte[] bytes) : base(bytes) {}
    public override Task<int> ReadAsync(byte[] buffer, int offset, int count, CancellationToken token) {
        if (Position > 0) throw new IOException("Simulated network interruption");
        return base.ReadAsync(buffer, offset, count, token);
    }
}
public class DownloadFixture : HttpMessageHandler {
    public string Scenario;
    public int Calls;
    public List<long> Offsets = new List<long>();
    public byte[] Payload = new byte[200000];
    public DownloadFixture(string scenario) {
        Scenario = scenario;
        for (int i = 0; i < Payload.Length; i++) Payload[i] = (byte)(i % 251);
    }
    protected override Task<HttpResponseMessage> SendAsync(HttpRequestMessage request, CancellationToken token) {
        Calls++;
        long offset = 0;
        if (request.Headers.Range != null) foreach (var range in request.Headers.Range.Ranges) offset = range.From.Value;
        Offsets.Add(offset);
        if (Scenario == "auth") return Task.FromResult(new HttpResponseMessage(HttpStatusCode.Unauthorized));
        if (Scenario == "transient" && Calls < 4) return Task.FromResult(new HttpResponseMessage(HttpStatusCode.ServiceUnavailable));
        bool broken = Calls == 1 && (Scenario == "resume" || Scenario == "ignore-range" || Scenario == "invalid-range");
        bool partial = offset > 0 && Scenario != "ignore-range";
        if (!partial) offset = 0;
        var bytes = new byte[Payload.Length - offset];
        Array.Copy(Payload, offset, bytes, 0, bytes.Length);
        var response = new HttpResponseMessage(partial ? HttpStatusCode.PartialContent : HttpStatusCode.OK);
        response.Content = broken ? (HttpContent)new StreamContent(new BrokenDownload(bytes)) : new ByteArrayContent(bytes);
        response.Content.Headers.ContentLength = bytes.Length;
        if (partial) response.Content.Headers.ContentRange = new ContentRangeHeaderValue(offset + (Scenario == "invalid-range" ? 1 : 0), Payload.Length - 1, Payload.Length);
        return Task.FromResult(response);
    }
}
'@
$tokens=$null; $errors=$null
$ast=[Management.Automation.Language.Parser]::ParseFile((Join-Path $PSScriptRoot 'runner-install.ps1'),[ref]$tokens,[ref]$errors)
if ($errors.Count) { throw ($errors | Out-String) }
$function=$ast.Find({ param($node) $node -is [Management.Automation.Language.FunctionDefinitionAst] -and $node.Name -eq 'Save-Download' },$true)
if (-not $function) { throw 'Download function not found' }
. ([ScriptBlock]::Create($function.Extent.Text))
# Skip real backoff in the fixture while executing all production retry logic.
function Start-Sleep { param([int]$Seconds) }
$root=Join-Path ([IO.Path]::GetTempPath()) ('runner-download-test-'+[Guid]::NewGuid().ToString('N'))
$null=New-Item -ItemType Directory -Path $root
$previousToken=$env:FERRY_TOKEN
$env:FERRY_TOKEN='fixture-only'
try {
    foreach ($scenario in @('resume','ignore-range','invalid-range','transient','auth')) {
        $handler=[DownloadFixture]::new($scenario)
        $client=[Net.Http.HttpClient]::new($handler)
        $path=Join-Path $root $scenario
        $failed=$false
        try {
            Save-Download 'https://fixture.invalid/runner' $path $handler.Payload.Length ($scenario -eq 'auth')
        } catch {
            $failed=$true
            if ($scenario -eq 'invalid-range' -and $_.ToString() -notlike '*Content-Range*') { throw }
            if ($scenario -eq 'auth' -and $_.ToString() -notlike '*HTTP 401*') { throw }
            if ($scenario -notin @('invalid-range','auth')) { throw }
        } finally { $client.Dispose() }
        if ($failed -ne ($scenario -in @('invalid-range','auth'))) { throw "Unexpected result: $scenario" }
        if (-not $failed) {
            $actual=[IO.File]::ReadAllBytes($path)
            if ([Convert]::ToBase64String($actual) -ne [Convert]::ToBase64String($handler.Payload)) { throw "Corrupt resumed output: $scenario" }
        }
        if ($scenario -in @('resume','ignore-range','invalid-range') -and $handler.Offsets[1] -ne 65536) { throw 'Did not resume at saved byte offset' }
        if ($scenario -eq 'transient' -and $handler.Calls -ne 4) { throw 'Transient retry stopped early' }
        if ($scenario -eq 'auth' -and $handler.Calls -ne 1) { throw 'Invalid token must not retry' }
        Write-Output "PASS $scenario"
    }
} finally {
    $env:FERRY_TOKEN=$previousToken
    Remove-Item -LiteralPath $root -Recurse -Force
}
