package main

import (
	"archive/zip"
	"bytes"
	"compress/flate"
	"crypto/rand"
	"crypto/sha256"
	"encoding/hex"
	"encoding/json"
	"fmt"
	"io"
	"io/fs"
	"net/http"
	"net/url"
	"os"
	"path/filepath"
	"slices"
	"strconv"
	"strings"
	"time"

	"github.com/bmatcuk/doublestar/v4"
	"github.com/golang-jwt/jwt/v5"
)

const artifactInputPrefix = "INPUT_FERRY_ARTIFACT_"

func artifactInput(name string) string {
	return os.Getenv(artifactInputPrefix + strings.ToUpper(strings.ReplaceAll(name, "-", "_")))
}

// The helper is a client of the existing authenticated artifact server; run
// scoping, duplicate detection, deletion and retention remain server concerns.
type artifactClient struct {
	endpoint, token, run, job string
	http                      *http.Client
}

func runArtifactHelper(operation string) error {
	c := &artifactClient{endpoint: strings.TrimRight(os.Getenv("ACTIONS_RUNTIME_URL"), "/"), token: os.Getenv("ACTIONS_RUNTIME_TOKEN"), http: &http.Client{}}
	claims := jwt.MapClaims{}
	if _, _, err := jwt.NewParser().ParseUnverified(c.token, claims); err != nil {
		return fmt.Errorf("artifact runtime token is missing or invalid")
	}
	scope, _ := claims["scp"].(string)
	for _, field := range strings.Fields(scope) {
		parts := strings.Split(field, ":")
		if len(parts) == 3 && parts[0] == "Actions.Results" {
			c.run, c.job = parts[1], parts[2]
			break
		}
	}
	if c.endpoint == "" || c.run == "" || c.job == "" {
		return fmt.Errorf("artifact runtime configuration is missing or invalid")
	}
	switch operation {
	case "upload":
		return c.upload()
	case "download":
		return c.download()
	default:
		return fmt.Errorf("unknown artifact helper operation")
	}
}
func (c *artifactClient) call(operation string, fields map[string]any, reply any) error {
	fields["workflowRunBackendId"], fields["workflowJobRunBackendId"] = c.run, c.job
	data, err := json.Marshal(fields)
	if err != nil {
		return fmt.Errorf("cannot encode artifact %s request", operation)
	}
	req, err := http.NewRequest(http.MethodPost, c.endpoint+"/twirp/github.actions.results.api.v1.ArtifactService/"+operation, bytes.NewReader(data))
	if err != nil {
		return fmt.Errorf("cannot construct artifact %s request", operation)
	}
	req.Header.Set("Content-Type", "application/json")
	req.Header.Set("Authorization", "Bearer "+c.token)
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("artifact %s request failed", operation)
	}
	defer resp.Body.Close()
	// Never echo URLs or response bodies: they can carry runtime capabilities.
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		if resp.StatusCode == http.StatusConflict {
			return fmt.Errorf("artifact already exists; use overwrite: true or a distinct name")
		}
		return fmt.Errorf("artifact %s failed (HTTP %d)", operation, resp.StatusCode)
	}
	if json.NewDecoder(resp.Body).Decode(reply) != nil {
		return fmt.Errorf("invalid artifact %s response", operation)
	}
	return nil
}
func artifactBool(name string) (bool, error) {
	switch strings.ToLower(strings.TrimSpace(artifactInput(name))) {
	case "", "false":
		return false, nil
	case "true":
		return true, nil
	default:
		return false, fmt.Errorf("%s must be true or false", name)
	}
}
func artifactInt(name string, fallback, maximum int) (int, error) {
	text := strings.TrimSpace(artifactInput(name))
	if text == "" {
		return fallback, nil
	}
	n, err := strconv.Atoi(text)
	if err != nil || n < 0 || n > maximum {
		return 0, fmt.Errorf("%s must be an integer between 0 and %d", name, maximum)
	}
	return n, nil
}
func artifactOutput(name, value string) error {
	f, err := os.OpenFile(os.Getenv("GITHUB_OUTPUT"), os.O_WRONLY|os.O_APPEND, 0600)
	if err != nil {
		return fmt.Errorf("cannot open artifact step output file")
	}
	defer f.Close()
	var nonce [16]byte
	if _, err := rand.Read(nonce[:]); err != nil {
		return fmt.Errorf("cannot create artifact output delimiter")
	}
	delimiter := "artifact_" + hex.EncodeToString(nonce[:])
	if _, err := fmt.Fprintf(f, "%s<<%s\n%s\n%s\n", name, delimiter, value, delimiter); err != nil {
		return fmt.Errorf("cannot write artifact step output")
	}
	return nil
}
func artifactExpandHome(path string) (string, error) {
	if path == "~" || strings.HasPrefix(path, "~/") || strings.HasPrefix(path, "~\\") {
		home, err := os.UserHomeDir()
		if err != nil {
			return "", fmt.Errorf("cannot resolve artifact home directory")
		}
		return filepath.Join(home, strings.TrimLeft(path[1:], "/\\")), nil
	}
	return path, nil
}

// Stop before the first wildcard, matching upload-artifact's preserved hierarchy.
func artifactSearchRoot(pattern string) string {
	volume := filepath.VolumeName(pattern)
	rest := strings.TrimPrefix(filepath.ToSlash(pattern), filepath.ToSlash(volume))
	parts := strings.Split(rest, "/")
	for i, part := range parts {
		if strings.ContainsAny(part, "*?[{") {
			prefix := volume + strings.Join(parts[:i], "/")
			if prefix == volume {
				prefix += "/"
			}
			return filepath.FromSlash(prefix)
		}
	}
	if info, err := os.Stat(pattern); err == nil && info.IsDir() {
		return pattern
	}
	return filepath.Dir(pattern)
}
func artifactHidden(path, explicit string) bool {
	parts := strings.Split(filepath.ToSlash(path), "/")
	named := strings.Split(filepath.ToSlash(explicit), "/")
	for i, part := range parts {
		if strings.HasPrefix(part, ".") && part != "." && part != ".." {
			explicitlyNamed := false
			for j, segment := range named {
				if !strings.HasPrefix(segment, ".") {
					continue
				}
				matched, _ := doublestar.Match(strings.Join(named[:j+1], "/"), strings.Join(parts[:i+1], "/"))
				if matched {
					explicitlyNamed = true
					break
				}
			}
			if !explicitlyNamed {
				return true
			}
		}
	}
	return false
}
func artifactFiles(input string, hidden bool) ([]string, string, error) {
	var includes, excludes, roots []string
	for _, line := range strings.Split(strings.ReplaceAll(input, "\r\n", "\n"), "\n") {
		line = strings.TrimSpace(line)
		if line == "" {
			continue
		}
		exclude := strings.HasPrefix(line, "!")
		if exclude {
			line = strings.TrimPrefix(line, "!")
		}
		expanded, err := artifactExpandHome(line)
		if err != nil {
			return nil, "", err
		}
		absolute, err := filepath.Abs(expanded)
		if err != nil {
			return nil, "", fmt.Errorf("cannot resolve artifact search path")
		}
		if exclude {
			excludes = append(excludes, filepath.ToSlash(absolute))
		} else {
			includes = append(includes, absolute)
			roots = append(roots, artifactSearchRoot(absolute))
		}
	}
	if len(includes) == 0 {
		return nil, "", fmt.Errorf("path input is required")
	}
	root := roots[0]
	for _, next := range roots[1:] {
		for {
			rel, err := filepath.Rel(root, next)
			if err == nil && rel != ".." && !strings.HasPrefix(rel, ".."+string(filepath.Separator)) {
				break
			}
			parent := filepath.Dir(root)
			if parent == root {
				return nil, "", fmt.Errorf("artifact search paths have no common root")
			}
			root = parent
		}
	}
	selected := map[string]bool{}
	for _, pattern := range includes {
		matches, err := doublestar.FilepathGlob(pattern, doublestar.WithFailOnIOErrors())
		if err != nil {
			return nil, "", fmt.Errorf("cannot search artifact path")
		}
		for _, match := range matches {
			err := filepath.WalkDir(match, func(path string, entry fs.DirEntry, walkErr error) error {
				if walkErr != nil {
					return walkErr
				}
				if !hidden && artifactHidden(path, pattern) {
					if entry.IsDir() {
						return filepath.SkipDir
					}
					return nil
				}
				for _, exclude := range excludes {
					matched := false
					for candidate := path; ; candidate = filepath.Dir(candidate) {
						var err error
						matched, err = doublestar.Match(exclude, filepath.ToSlash(candidate))
						if err != nil {
							return err
						}
						if matched || filepath.Dir(candidate) == candidate {
							break
						}
					}
					if matched {
						if entry.IsDir() {
							return filepath.SkipDir
						}
						return nil
					}
				}
				if !entry.IsDir() {
					info, err := os.Stat(path)
					if err != nil {
						return err
					}
					if info.Mode().IsRegular() {
						selected[path] = true
					}
				}
				return nil
			})
			if err != nil {
				return nil, "", fmt.Errorf("cannot read artifact search path")
			}
		}
	}
	files := make([]string, 0, len(selected))
	for path := range selected {
		files = append(files, path)
	}
	slices.Sort(files)
	return files, root, nil
}
func artifactZIP(files []string, root string, level int) (*os.File, string, error) {
	f, err := os.CreateTemp("", "artifact-*.zip")
	if err != nil {
		return nil, "", fmt.Errorf("cannot create artifact archive")
	}
	ok := false
	defer func() {
		if !ok {
			f.Close()
			os.Remove(f.Name())
		}
	}()
	digest := sha256.New()
	archive := zip.NewWriter(io.MultiWriter(f, digest))
	archive.RegisterCompressor(zip.Deflate, func(w io.Writer) (io.WriteCloser, error) { return flate.NewWriter(w, level) })
	for _, path := range files {
		if err := artifactZIPFile(archive, path, root); err != nil {
			archive.Close()
			return nil, "", err
		}
	}
	if archive.Close() != nil {
		return nil, "", fmt.Errorf("cannot finish artifact archive")
	}
	if _, err := f.Seek(0, io.SeekStart); err != nil {
		return nil, "", fmt.Errorf("cannot read artifact archive")
	}
	ok = true
	return f, hex.EncodeToString(digest.Sum(nil)), nil
}
func artifactZIPFile(archive *zip.Writer, path, root string) error {
	f, err := os.Open(path)
	if err != nil {
		return fmt.Errorf("cannot open artifact file")
	}
	defer f.Close()
	info, err := f.Stat()
	if err != nil {
		return fmt.Errorf("cannot inspect artifact file")
	}
	header, err := zip.FileInfoHeader(info)
	if err != nil {
		return fmt.Errorf("cannot create artifact archive entry")
	}
	rel, err := filepath.Rel(root, path)
	if err != nil || !filepath.IsLocal(rel) {
		return fmt.Errorf("artifact file is outside archive root")
	}
	header.Name, header.Method = filepath.ToSlash(rel), zip.Deflate
	header.SetMode(info.Mode())
	writer, err := archive.CreateHeader(header)
	if err != nil {
		return fmt.Errorf("cannot create artifact archive entry")
	}
	if _, err := io.Copy(writer, f); err != nil {
		return fmt.Errorf("cannot archive artifact file")
	}
	return nil
}
func (c *artifactClient) upload() error {
	name := artifactInput("name")
	if name == "" {
		name = "artifact"
	}
	level, err := artifactInt("compression-level", 6, 9)
	if err != nil {
		return err
	}
	days, err := artifactInt("retention-days", 0, int((1<<63-1)/int64(24*time.Hour)))
	if err != nil {
		return err
	}
	hidden, err := artifactBool("include-hidden-files")
	if err != nil {
		return err
	}
	overwrite, err := artifactBool("overwrite")
	if err != nil {
		return err
	}
	noFiles := artifactInput("if-no-files-found")
	if noFiles == "" {
		noFiles = "warn"
	}
	if noFiles != "warn" && noFiles != "error" && noFiles != "ignore" {
		return fmt.Errorf("if-no-files-found must be warn, error or ignore")
	}
	files, root, err := artifactFiles(artifactInput("path"), hidden)
	if err != nil {
		return err
	}
	if len(files) == 0 {
		if noFiles == "error" {
			return fmt.Errorf("no files found for artifact path")
		}
		if noFiles == "warn" {
			fmt.Println("::warning::No files found for artifact path; no artifact uploaded")
		}
		return nil
	}
	archive, digest, err := artifactZIP(files, root, level)
	if err != nil {
		return err
	}
	defer os.Remove(archive.Name())
	defer archive.Close()
	if overwrite {
		var deleted struct {
			OK bool `json:"ok"`
		}
		if err := c.call("DeleteArtifact", map[string]any{"name": name}, &deleted); err != nil {
			return err
		}
		if !deleted.OK {
			return fmt.Errorf("artifact DeleteArtifact was not accepted")
		}
	}
	create := map[string]any{"name": name, "version": 4}
	if days > 0 {
		create["expiresAt"] = time.Now().Add(time.Duration(days) * 24 * time.Hour).UTC().Format(time.RFC3339Nano)
	}
	var created struct {
		OK  bool   `json:"ok"`
		URL string `json:"signedUploadUrl"`
	}
	if err := c.call("CreateArtifact", create, &created); err != nil {
		return err
	}
	if !created.OK || created.URL == "" {
		return fmt.Errorf("artifact CreateArtifact was not accepted")
	}
	link, err := url.Parse(created.URL)
	if err != nil {
		return fmt.Errorf("invalid artifact upload URL")
	}
	query := link.Query()
	query.Set("comp", "block")
	link.RawQuery = query.Encode()
	req, err := http.NewRequest(http.MethodPut, link.String(), archive)
	if err != nil {
		return fmt.Errorf("cannot construct artifact upload request")
	}
	req.Header.Set("Content-Type", "application/zip")
	info, err := archive.Stat()
	if err != nil {
		return fmt.Errorf("cannot inspect artifact archive")
	}
	req.ContentLength = info.Size()
	resp, err := c.http.Do(req)
	if err != nil {
		return fmt.Errorf("artifact blob upload failed")
	}
	resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("artifact blob upload failed (HTTP %d)", resp.StatusCode)
	}
	var finalized struct {
		OK bool   `json:"ok"`
		ID string `json:"artifactId"`
	}
	if err := c.call("FinalizeArtifact", map[string]any{"name": name, "size": strconv.FormatInt(info.Size(), 10), "hash": "sha256:" + digest}, &finalized); err != nil {
		return err
	}
	if !finalized.OK || finalized.ID == "" {
		return fmt.Errorf("artifact FinalizeArtifact was not accepted")
	}
	for _, output := range [][2]string{{"artifact-id", finalized.ID}, {"artifact-url", ""}, {"artifact-digest", digest}} {
		if err := artifactOutput(output[0], output[1]); err != nil {
			return err
		}
	}
	return nil
}
func (c *artifactClient) download() error {
	for _, input := range []string{"github-token", "repository", "run-id"} {
		if _, present := os.LookupEnv(artifactInputPrefix + strings.ToUpper(strings.ReplaceAll(input, "-", "_"))); present {
			return fmt.Errorf("download-artifact input %s is not supported by the local artifact server", input)
		}
	}
	merge, err := artifactBool("merge-multiple")
	if err != nil {
		return err
	}
	destination := artifactInput("path")
	if destination == "" {
		destination = os.Getenv("GITHUB_WORKSPACE")
	}
	destination, err = artifactExpandHome(destination)
	if err != nil {
		return err
	}
	destination, err = filepath.Abs(destination)
	if err != nil {
		return fmt.Errorf("cannot resolve artifact download path")
	}
	if os.MkdirAll(destination, 0755) != nil {
		return fmt.Errorf("cannot create artifact download directory")
	}
	root, err := os.OpenRoot(destination)
	if err != nil {
		return fmt.Errorf("cannot open artifact download directory")
	}
	defer root.Close()
	name, pattern := artifactInput("name"), artifactInput("pattern")
	var listed struct {
		Artifacts []struct {
			Name string `json:"name"`
		} `json:"artifacts"`
	}
	fields := map[string]any{}
	if name != "" {
		fields["nameFilter"] = name
	}
	if err := c.call("ListArtifacts", fields, &listed); err != nil {
		return err
	}
	found := false
	for _, artifact := range listed.Artifacts {
		if name != "" && artifact.Name != name {
			continue
		}
		if name == "" && pattern != "" {
			matched, err := doublestar.Match(pattern, artifact.Name)
			if err != nil {
				return fmt.Errorf("invalid artifact pattern")
			}
			if !matched {
				continue
			}
		}
		if !fs.ValidPath(artifact.Name) || strings.ContainsAny(artifact.Name, "/\\") || artifact.Name == "." {
			return fmt.Errorf("invalid artifact name in server response")
		}
		found = true
		subdir := "."
		if name == "" && !merge {
			subdir = artifact.Name
		}
		if err := c.downloadOne(root, subdir, artifact.Name); err != nil {
			return err
		}
	}
	if name != "" && !found {
		return fmt.Errorf("requested artifact was not found")
	}
	return artifactOutput("download-path", destination)
}
func (c *artifactClient) downloadOne(root *os.Root, directory, name string) error {
	var signed struct {
		URL string `json:"signedUrl"`
	}
	if err := c.call("GetSignedArtifactURL", map[string]any{"name": name}, &signed); err != nil {
		return err
	}
	resp, err := c.http.Get(signed.URL)
	if err != nil {
		return fmt.Errorf("artifact blob download failed")
	}
	defer resp.Body.Close()
	if resp.StatusCode < 200 || resp.StatusCode >= 300 {
		return fmt.Errorf("artifact blob download failed (HTTP %d)", resp.StatusCode)
	}
	f, err := os.CreateTemp("", "artifact-download-*.zip")
	if err != nil {
		return fmt.Errorf("cannot create temporary artifact download")
	}
	defer os.Remove(f.Name())
	defer f.Close()
	if _, err := io.Copy(f, resp.Body); err != nil {
		return fmt.Errorf("cannot download artifact archive")
	}
	info, err := f.Stat()
	if err != nil {
		return fmt.Errorf("cannot inspect downloaded artifact")
	}
	archive, err := zip.NewReader(f, info.Size())
	if err != nil {
		return fmt.Errorf("downloaded artifact is not a valid ZIP archive")
	}
	for _, entry := range archive.File {
		if err := artifactExtractFile(root, directory, entry); err != nil {
			return err
		}
	}
	return nil
}
func artifactExtractFile(root *os.Root, directory string, entry *zip.File) error {
	name := strings.TrimSuffix(entry.Name, "/")
	if !fs.ValidPath(name) || !filepath.IsLocal(name) || strings.Contains(name, "\\") {
		return fmt.Errorf("artifact archive contains an unsafe path")
	}
	if entry.Mode()&os.ModeSymlink != 0 {
		return fmt.Errorf("artifact archive contains an unsupported symbolic link")
	}
	path := filepath.Join(directory, filepath.FromSlash(name))
	if entry.FileInfo().IsDir() {
		if root.MkdirAll(path, 0755) != nil {
			return fmt.Errorf("cannot create artifact directory")
		}
		return nil
	}
	if root.MkdirAll(filepath.Dir(path), 0755) != nil {
		return fmt.Errorf("cannot create artifact directory")
	}
	source, err := entry.Open()
	if err != nil {
		return fmt.Errorf("cannot read artifact archive entry")
	}
	defer source.Close()
	mode := entry.Mode().Perm()
	if mode == 0 {
		mode = 0644
	}
	f, err := root.OpenFile(path, os.O_WRONLY|os.O_CREATE|os.O_TRUNC, mode)
	if err != nil {
		return fmt.Errorf("cannot create downloaded artifact file")
	}
	defer f.Close()
	if _, err := io.Copy(f, source); err != nil {
		return fmt.Errorf("cannot extract artifact file")
	}
	if f.Chmod(mode) != nil {
		return fmt.Errorf("cannot restore artifact file permissions")
	}
	return nil
}
