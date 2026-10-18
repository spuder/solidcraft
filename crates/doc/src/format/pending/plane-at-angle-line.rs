//! A Plane at Angle can turn about model geometry (`line`: a straight edge, sketch line or
//! construction axis), found again on every evaluation so the plane follows it, and goes
//! through that line at the angle to its base (#2229); `axis_origin` and `axis_dir` keep where
//! it was when picked. Older designs need no
//! transformation: a plane without `line` rotates about its fixed `axis_origin`, `axis_dir` as
//! before; 0.15.x asks for an update instead of rotating about the stale picked line.
