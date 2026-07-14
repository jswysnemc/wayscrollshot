use opencv::{not_opencv_branch_5, opencv_branch_5};

opencv_branch_5! {
    pub(crate) use opencv::geometry::{estimate_affine_partial_2d, RANSAC};
}

not_opencv_branch_5! {
    pub(crate) use opencv::calib3d::{estimate_affine_partial_2d, RANSAC};
}
