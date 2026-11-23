const registration = '/System/Library/Frameworks/CoreServices.framework/Frameworks/LaunchServices.framework/Support/lsregister'

export def prepare [out: path] {
    let root = $out | path join fixtures
    if ($root | path exists) { error make {msg: 'File-action fixtures already exist'} }
    let bundle = $root | path join DupinE2EViewer.app
    mkdir ($bundle | path join Contents MacOS)
    cp ($env.FILE_PWD | path join fixtures FileViewer.plist) ($bundle | path join Contents Info.plist)
    ^swiftc -framework AppKit ($env.FILE_PWD | path join fixtures FileViewer.swift) -o ($bundle | path join Contents MacOS viewer)
    ^codesign --force --sign - $bundle
    ^$registration -f $bundle
    for name in [open-me.dupine2e trash-me.dupine2e] {
        $'Dupin disposable fixture: ($root)/($name)' | save ($root | path join $name)
    }
    open --raw ($root | path join trash-me.dupine2e) | hash sha256 | save ($root | path join trash.sha256)
    ^mdimport $root
    for _ in 1..30 {
        let indexed = ^mdfind -onlyin $root 'kMDItemFSName == "*.dupine2e"' | lines
        if ($indexed | all {|path| $path | path exists }) and ($indexed | length) == 2 { return }
        sleep 1sec
    }
    error make {msg: 'Spotlight did not index both file-action fixtures'}
}

export def cleanup [out: path] {
    let bundle = $out | path join fixtures DupinE2EViewer.app
    ^osascript -e 'if application id "org.dupin.e2e.viewer" is running then tell application id "org.dupin.e2e.viewer" to quit'
    ^$registration -u $bundle
    let expected = open --raw ($out | path join fixtures trash.sha256)
    for path in (glob ($env.HOME | path join '.Trash/trash-me*.dupine2e')) {
        if (open --raw $path | hash sha256) != $expected {
            error make {msg: 'Refusing to remove an unexpected Trash item'}
        }
        rm $path
    }
}
