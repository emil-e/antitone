
// Declaration of all textures

layout(set=0, binding = 1) uniform sampler2D aces_reach_m_table_0Sampler; 
layout(set=0, binding = 2) uniform sampler2D aces_gamut_cusp_table_0Sampler; 

// Declaration of all helper methods

float aces_reach_m_table_0_sample(float h)
{
  float i_base = floor(h);
  float i_lo = i_base + float(1);
  float i_hi = i_lo + 1.0;
  float lo = texture(aces_reach_m_table_0Sampler, vec2((i_lo + 0.5) / float (363), 0.0)).r;
  float hi = texture(aces_reach_m_table_0Sampler, vec2((i_hi + 0.5) / float (363), 0.5)).r;
  float t = h - i_base;
  return mix(lo, hi, t);
}
float aces_tonescale_fwd0(float J)
{
  float A = 0.0323680267 * pow(abs(J) * 0.00999999978, 0.879464149);
  float Y = pow(( 27.1299992 * A) / (1.0f - A), 2.3809523809523809);
  float f = 1.04710376 * pow(Y / (Y + 0.73009213709383403), 1.14999998);
  float Y_ts = max(0.0, f * f / (f + 0.0399999991));
  float F_L_Y = pow(0.79370057210326195 * Y_ts, 0.42);
  float J_ts = 100. * pow((F_L_Y / ( 27.1299992 + F_L_Y)) * 30.8946857, 1.13705599);
  return sign(J) * J_ts;
}
float aces_toe_fwd0(float x, float limit, float k1_in, float k2_in)
{
  float k2 = max(k2_in, 0.001);
  float k1 = sqrt(k1_in * k1_in + k2 * k2);
  float k3 = (limit + k1) / (limit + k2);
  return (x > limit) ? x : 0.5 * (k3 * x - k1 + sqrt((k3 * x - k1) * (k3 * x - k1) + 4.0 * k2 * k3 * x));
}
const float aces_gamut_cusp_table_0_hues_array[363] = float[363](-1.01858521, 0., 0.999435902, 1.9988718, 2.9983077, 3.99774361, 4.99717951, 5.99661541, 6.99605131, 7.99548721, 8.99492264, 9.99435902, 10.9937954, 11.9932308, 12.9926662, 13.9921026, 14.991539, 15.9909744, 16.9904099, 17.9898453, 18.9892826, 19.988718, 20.9881535, 21.9875908, 22.9870262, 23.9864616, 24.9858971, 26.0706215, 27.1553478, 28.2400723, 29.2381344, 30.2361965, 31.2342587, 32.2323227, 33.2303848, 34.228447, 35.2265091, 36.2245712, 37.2226334, 38.2206955, 39.2187576, 40.2168198, 41.2148819, 42.212944, 43.2110062, 44.2090683, 45.2071304, 46.2051926, 47.2032547, 48.2013168, 49.199379, 50.1974411, 51.1955032, 52.1935654, 53.1916275, 54.1896896, 55.1877518, 56.1858139, 57.183876, 58.1819382, 59.1800003, 60.1780624, 61.1761284, 62.1741905, 63.1722527, 64.170311, 65.1683731, 66.1664352, 67.1644974, 68.1625595, 69.1606216, 70.1586838, 71.1567459, 72.154808, 73.1528702, 74.1509323, 75.1489944, 76.1470642, 77.1451263, 78.1431885, 79.1412506, 80.1393127, 81.1373749, 82.135437, 83.1334991, 84.1315613, 85.1296234, 86.1276855, 87.1257477, 88.1238098, 89.1218719, 90.1199341, 91.1179962, 92.1160583, 93.1141205, 94.1121826, 95.1102448, 96.1083069, 97.106369, 98.1044312, 99.1024933, 100.100555, 101.098618, 102.09668, 103.094742, 104.092804, 105.090866, 106.088928, 107.194717, 108.188995, 109.183273, 110.177544, 111.171822, 112.1661, 113.160378, 114.154648, 115.148926, 116.143204, 117.137482, 118.13176, 119.12603, 120.120308, 121.114586, 122.108864, 123.103134, 124.097412, 125.09169, 126.085968, 127.080238, 128.074524, 129.068802, 130.063065, 131.057343, 132.05162, 133.045898, 134.040176, 135.034454, 136.028732, 137.02301, 138.017273, 139.011551, 140.005829, 141.000107, 141.994385, 143.037018, 144.079651, 145.122299, 146.164932, 147.207565, 148.207993, 149.20842, 150.208847, 151.209274, 152.209702, 153.210129, 154.210556, 155.210983, 156.211411, 157.211838, 158.212265, 159.212692, 160.21312, 161.213547, 162.213974, 163.214401, 164.214828, 165.215256, 166.215683, 167.21611, 168.216537, 169.216965, 170.217392, 171.217819, 172.218246, 173.218674, 174.219101, 175.219528, 176.219955, 177.220383, 178.22081, 179.221237, 180.221664, 181.222092, 182.222519, 183.222946, 184.223373, 185.223801, 186.224228, 187.224655, 188.225082, 189.22551, 190.225937, 191.226364, 192.226791, 193.031708, 193.836639, 194.641556, 195.650192, 196.658829, 197.667465, 198.676117, 199.684753, 200.69339, 201.702026, 202.710663, 203.719299, 204.727936, 205.736572, 206.745209, 207.75386, 208.762497, 209.771133, 210.77977, 211.788406, 212.797043, 213.805679, 214.814331, 215.822968, 216.831604, 217.84024, 218.848877, 219.857513, 220.86615, 221.874786, 222.883423, 223.892075, 224.900711, 225.909348, 226.917984, 227.92662, 228.935257, 229.943893, 230.952545, 231.961182, 232.969818, 233.978455, 234.987091, 235.995728, 237.004364, 238.013, 239.021637, 240.030289, 241.038925, 242.047562, 243.056198, 244.064835, 245.073471, 246.082108, 247.090759, 248.099396, 249.108032, 250.116669, 251.125305, 252.133942, 253.142578, 254.151215, 255.159851, 256.168488, 257.177124, 258.18576, 259.194397, 260.203064, 261.2117, 262.220337, 263.228973, 264.23761, 265.246246, 266.254883, 267.263519, 268.272156, 269.26947, 270.266785, 271.264069, 272.261383, 273.258698, 274.255981, 275.253296, 276.25061, 277.246796, 278.24295, 279.239136, 280.235321, 281.231476, 282.227661, 283.223846, 284.220001, 285.216187, 286.212372, 287.208557, 288.204712, 289.200897, 290.197083, 291.193237, 292.189423, 293.185608, 294.181763, 295.177948, 296.174133, 297.170288, 298.166473, 299.162659, 300.158813, 301.154999, 302.151184, 303.147339, 304.143524, 305.139709, 306.135895, 307.13205, 308.128235, 309.12442, 310.120575, 311.11676, 312.112946, 313.1091, 314.105286, 315.101471, 316.097626, 317.093811, 318.089996, 319.086151, 320.082336, 321.078522, 322.074677, 323.070862, 324.067047, 325.063202, 326.059387, 327.055573, 328.051758, 329.047913, 330.044098, 331.040283, 332.036438, 333.032623, 334.028809, 335.024963, 336.021149, 336.961853, 337.902557, 338.843262, 339.783966, 340.72467, 341.665375, 342.68396, 343.702545, 344.721161, 345.739746, 346.758331, 347.776917, 348.795502, 349.814087, 350.832703, 351.851288, 352.869873, 353.888458, 354.907043, 355.925629, 356.944244, 357.96283, 358.981415, 360., 360.999451);
vec3 aces_gamut_cusp_table_0_sample(float h)
{
  int i = int(h) + 1;
  int i_lo = int(max(float(0), float(i + 0)));
  int i_hi = int(min(float(361), float(i + 2)));
  while (i_lo + 1 < i_hi)
  {
    float hcur = aces_gamut_cusp_table_0_hues_array[i];
    if (h > hcur)
    {
      i_lo = i;
    }
    else
    {
      i_hi = i;
    }
    i = (i_lo + i_hi) / 2;
  }
  vec3 lo = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) - 1.0 + 0.5) / float(363), 0.5)).rgb;
  vec3 hi = texture(aces_gamut_cusp_table_0Sampler, vec2((float(i_hi) + 0.5) / float(363), 0.5)).rgb;
  float t = (h - aces_gamut_cusp_table_0_hues_array[i_hi - 1]) / (aces_gamut_cusp_table_0_hues_array[i_hi] - aces_gamut_cusp_table_0_hues_array[i_hi - 1]);
  return mix(lo, hi, t);
}
float aces_get_focus_gain0(float J, float cuspJ)
{
  float thr = mix(cuspJ, 100.000000, 0.300000);
  if (J > thr)
  {
    float gain = ( 100. - thr) / max(0.0001, 100. - J);
    gain = log(gain)/log(10.0);
    return gain * gain + 1.0;
  }
  else
  {
    return 1.0;
  }
}
float aces_solve_J_intersect0(float J, float M, float focusJ, float slope_gain)
{
  float M_scaled = M / slope_gain;
  float a = M_scaled / focusJ;
  if (J < focusJ)
  {
    float b = 1.0 - M_scaled;
    float c = -J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b + root);
  }
  else
  {
    float b = - (1.0 + M_scaled + 100. * a);
    float c = 100. * M_scaled + J;
    float det =  b * b - 4.f * a * c;
    float root =  sqrt(det);
    return -2.0 * c / (b - root);
  }
}
float aces_find_gamut_boundary_intersection0(vec2 JM_cusp, float gamma_top_inv, float gamma_bottom_inv, float J_intersect_source, float J_intersect_cusp, float slope)
{
  float M_boundary_lower = J_intersect_cusp * pow(J_intersect_source / J_intersect_cusp, gamma_bottom_inv) / (JM_cusp.r / JM_cusp.g - slope);
  float M_boundary_upper = JM_cusp.g * (100. - J_intersect_cusp) * pow((100. - J_intersect_source) / (100. - J_intersect_cusp), gamma_top_inv) / (slope * JM_cusp.g + 100. - JM_cusp.r);
  float smin = 0.0;
  {
    float a = M_boundary_lower;
    float b = M_boundary_upper;
    float s = 0.119999997 * JM_cusp.g;
    float h = max(s - abs(a - b), 0.0) / s;
    smin = min(a, b) - h * h * h * s * 0.16666666666666666;
  }
  return smin;
}
float aces_remap_M_fwd0(float M, float gamut_boundary_M, float reach_boundary_M)
{
  float boundary_ratio = gamut_boundary_M / reach_boundary_M;
  float proportion = max(boundary_ratio, 0.75);
  float threshold = proportion * gamut_boundary_M;
  if (proportion >= 1.0f || M <= threshold)
  {
    return M;
  }
  float m_offset = M - threshold;
  float gamut_offset = gamut_boundary_M - threshold;
  float reach_offset = reach_boundary_M - threshold;
  float scale = reach_offset / ((reach_offset / gamut_offset) - 1.0f);
  float nd = m_offset / scale;
  return threshold + scale * nd / (1.0f + nd);
}
vec3 aces_gamut_compress0(vec3 JMh, float Jx, vec3 JMGcusp, float reachMaxM)
{
  float J = JMh.r;
  float M = JMh.g;
  float h = JMh.b;
  if (M <= 0.0 || J > 100.)
  {
    return vec3(J, 0.0, h);
  }
  else
  {
    vec2 JMcusp = JMGcusp.rg;
    float focusJ = mix(JMcusp.r, 34.096539, min(1.0, 1.300000 - (JMcusp.r / 100.000000)));
    float slope_gain = 135. * aces_get_focus_gain0(Jx, JMcusp.r);
    float J_intersect_source = aces_solve_J_intersect0(JMh.r, JMh.g, focusJ, slope_gain);
    float gamut_slope = (J_intersect_source < focusJ) ? J_intersect_source : (100. - J_intersect_source);
    gamut_slope = gamut_slope * (J_intersect_source - focusJ) / (focusJ * slope_gain);
    float gamma_top_inv = JMGcusp.b;
    float gamma_bottom_inv = 0.877192974;
    float J_intersect_cusp = aces_solve_J_intersect0(JMcusp.r, JMcusp.g, focusJ, slope_gain);
    float gamutBoundaryM = aces_find_gamut_boundary_intersection0(JMcusp, gamma_top_inv, gamma_bottom_inv, J_intersect_source, J_intersect_cusp, gamut_slope);
    if (gamutBoundaryM <= 0.0)
    {
      return vec3(J, 0.0, h);
    }
    float reachBoundaryM = 100. * pow(J_intersect_source / 100.,  0.879464149);
    reachBoundaryM = reachBoundaryM / ((100. / reachMaxM) - gamut_slope);
    float remapped_M = aces_remap_M_fwd0(M, gamutBoundaryM, reachBoundaryM);
    float remapped_J = J_intersect_source + remapped_M * gamut_slope;
    return vec3(remapped_J, remapped_M, h);
  }
}

// Declaration of the OCIO shader function

vec4 antitone_render(vec4 inPixel)
{
  vec4 outColor = inPixel;
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1024., 1024., 1024.), outColor.rgb);
  }
  
  // Add Matrix processing
  
  {
    vec4 res = vec4(outColor.rgb.r, outColor.rgb.g, outColor.rgb.b, outColor.a);
    vec4 tmp = res;
    res = mat4(0.69545224135745187, 0.044794563372037723, -0.0055258825581135443, 0., 0.14067869647029416, 0.85967111845642175, 0.0040252103059786595, 0., 0.16386906217225405, 0.0955343181715404, 1.0015006722521349, 0., 0., 0., 0., 1.) * tmp;
    outColor.rgb = vec3(res.x, res.y, res.z);
    outColor.a = res.w;
  }
  
  // Add FixedFunction 'ACES_OutputTransform20 (Forward)' processing
  
  {
    
    // Add RGB to JMh
    
    vec3 JMh;
    vec3 Aab;
    {
      {
        vec3 lms = mat3(0.445181042, 0.123734146, 0.0117007261, 0.34964928, 0.613643706, 0.0280607939, -0.00112973328, 0.0563228093, 0.753939092) * outColor.rgb;
        vec3 F_L_v = pow(abs(lms), vec3(0.419999987, 0.419999987, 0.419999987));
        vec3 rgb_a = (sign(lms) * F_L_v) / ( 27.1299992 + F_L_v);
        Aab = mat3(20.2588081, 15480., 1720., 10.1294041, -16887.2734, 1720., 0.506470203, 1407.27271, -3440.) * rgb_a.rgb;
      }
      {
        if (Aab.r <= 0.0)
        {
          JMh.rgb = vec3(0., 0., 0.);
        }
        else
        {
          float J = 100. * pow(Aab.r, 1.13705599);
          float M = (J == 0.0) ? 0.0 : sqrt(Aab.g * Aab.g + Aab.b * Aab.b);
          float h = (Aab.g == 0.0) ? 0.0 : atan(Aab.b, Aab.g) * 57.29577951308238;
          h = h - floor(h / 360.0) * 360.0;
          h = (h < 0.0) ? h + 360.0 : h;
          JMh.rgb = vec3(J, M, h);
        }
      }
      outColor.rgb = JMh;
    }
    float h_rad = outColor.b * 0.0174532924;
    float cos_hr = cos(h_rad);
    float sin_hr = sin(h_rad);
    
    // Add ToneScale and ChromaCompress (fwd)
    
    float J_ts = aces_tonescale_fwd0(outColor.r);
    // Sample tables (fwd)
    float reachMaxM = aces_reach_m_table_0_sample(outColor.b);
    
    {
      float J = outColor.r;
      float M = outColor.g;
      float h = outColor.b;
      float M_cp = M;
      if (M != 0.0)
      {
        float nJ = J_ts / 100.;
        float snJ = max(0.0, 1.0 - nJ);
        float Mnorm;
        {
          float cos_hr2 = 2.0 * cos_hr * cos_hr - 1.0;
          float sin_hr2 = 2.0 * cos_hr * sin_hr;
          float cos_hr3 = 4.0 * cos_hr * cos_hr * cos_hr - 3.0 * cos_hr;
          float sin_hr3 = 3.0 * sin_hr - 4.0 * sin_hr * sin_hr * sin_hr;
          vec3 cosines = vec3(cos_hr, cos_hr2, cos_hr3);
          vec3 cosine_weights = vec3(11.341321604032515, 16.469863649185896, 7.8842182208776475);
          vec3 sines = vec3(sin_hr, sin_hr2, sin_hr3);
          vec3 sine_weights = vec3(14.665187919584513, -6.3725780354404442, 9.1941277054452897);
          Mnorm = dot(cosines, cosine_weights) + dot(sines, sine_weights) + 77.133051547393805;
        }
        float limit = pow(nJ, 0.879464149) * reachMaxM / Mnorm;
        M_cp = M * pow(J_ts / J, 0.879464149);
        M_cp = M_cp / Mnorm;
        M_cp = limit - aces_toe_fwd0(limit - M_cp, limit - 0.001, snJ * 1.29999995, sqrt(nJ * nJ + 0.00499999989));
        M_cp = aces_toe_fwd0(M_cp, limit, nJ * 2.4000001, snJ);
        M_cp = M_cp * Mnorm;
      }
      outColor.rgb = vec3(J_ts, M_cp, h);
    }
    
    // Add GamutCompress (fwd)
    
    {
      vec3 JMGcusp = aces_gamut_cusp_table_0_sample(outColor.b);
      outColor.rgb = aces_gamut_compress0(outColor.rgb, outColor.r, JMGcusp, reachMaxM);
    }
    
    // Add JMh to RGB
    
    {
      vec3 JMh = outColor.rgb;
      vec3 Aab;
      {
        Aab.r = pow(JMh.r * 0.00999999978, 0.879464149);
        Aab.g = JMh.g * cos_hr;
        Aab.b = JMh.g * sin_hr;
      }
      {
        vec3 rgb_a = mat3(0.0323680304, 0.0323680304, 0.0323680304, 2.07657631e-05, -4.10250432e-05, -1.01296409e-05, 1.3260621e-05, -1.20174373e-05, -0.000290076074) * Aab.rgb;
        vec3 rgb_a_lim = min( abs(rgb_a), vec3(0.99000001, 0.99000001, 0.99000001) );
        vec3 lms = sign(rgb_a) * pow( 27.1299992 * rgb_a_lim / (1.0f - rgb_a_lim), vec3(2.38095236, 2.38095236, 2.38095236));
        JMh.rgb = mat3(5.86586046, -1.17879081, 0.0301606376, -4.4882164, 2.81135964, -0.16902554, -0.117723353, -0.372647762, 1.39878595) * lms;
      }
      outColor.rgb = JMh;
    }
  }
  
  // Add Range processing
  
  {
    outColor.rgb = max(vec3(0., 0., 0.), outColor.rgb);
    outColor.rgb = min(vec3(1., 1., 1.), outColor.rgb);
  }

  return outColor;
}
